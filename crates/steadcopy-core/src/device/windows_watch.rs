//! Windows 的设备到达/移除监听：消息窗口 + 设备接口通知。
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//! → Requirement: 事件驱动的设备到达监听
//! 事实依据：`docs/source-devices.md` §八
//!
//! # 为什么要自己建窗口
//!
//! 卷到达（`DBT_DEVTYP_VOLUME`）是**自动广播给所有顶层窗口**的，
//! 但纯消息窗口（`HWND_MESSAGE`）不是顶层窗口，收不到这个广播。
//! 所以这里的做法是：建一个隐藏的顶层窗口（0×0、不进任务栏），
//! 它既能收到卷广播，又不打扰用户。
//!
//! # 噪声
//!
//! `DBT_DEVNODES_CHANGED`(0x0007) 在枚举期间会连发多次，**绝不当信号**。
//! 只认 `DBT_DEVICEARRIVAL`(0x8000) 与 `DBT_DEVICEREMOVECOMPLETE`(0x8004)，
//! 且只处理其中 `dbch_devicetype == DBT_DEVTYP_VOLUME` 的那些。

use std::sync::mpsc::{channel, Receiver, Sender};

use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostQuitMessage, RegisterClassW, TranslateMessage, CW_USEDEFAULT, MSG, WINDOW_EX_STYLE,
    WM_DESTROY, WM_DEVICECHANGE, WNDCLASSW, WS_OVERLAPPED,
};

use crate::device::watch::{drive_letters_from_mask, DeviceEvent, DeviceWatcher};
use crate::error::{CoreError, ErrorContext, Result, TerminalKind};

const DBT_DEVICEARRIVAL: usize = 0x8000;
const DBT_DEVICEREMOVECOMPLETE: usize = 0x8004;
const DBT_DEVTYP_VOLUME: u32 = 2;

#[repr(C)]
struct DevBroadcastHdr {
    size: u32,
    device_type: u32,
    reserved: u32,
}

#[repr(C)]
struct DevBroadcastVolume {
    size: u32,
    device_type: u32,
    reserved: u32,
    unit_mask: u32,
    flags: u16,
}

thread_local! {
    static SENDER: std::cell::RefCell<Option<Sender<DeviceEvent>>> =
        const { std::cell::RefCell::new(None) };
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_DEVICECHANGE {
        // 只认到达与移除完成两种，其余（含 DBT_DEVNODES_CHANGED 噪声）一律忽略
        let arrived = wparam.0 == DBT_DEVICEARRIVAL;
        let removed = wparam.0 == DBT_DEVICEREMOVECOMPLETE;
        if (arrived || removed) && lparam.0 != 0 {
            // SAFETY: 系统保证 lParam 指向一个 DEV_BROADCAST_HDR
            let hdr = unsafe { &*(lparam.0 as *const DevBroadcastHdr) };
            if hdr.device_type == DBT_DEVTYP_VOLUME {
                // SAFETY: device_type 已确认为卷类型，可安全按卷结构读取
                let vol = unsafe { &*(lparam.0 as *const DevBroadcastVolume) };
                let letters = drive_letters_from_mask(vol.unit_mask);
                SENDER.with(|s| {
                    if let Some(tx) = s.borrow().as_ref() {
                        if letters.is_empty() {
                            // 无盘符的卷：也要通知，由消费方重新枚举去认领
                            let _ = tx.send(if arrived {
                                DeviceEvent::Arrived { drive_letter: None }
                            } else {
                                DeviceEvent::Removed { drive_letter: None }
                            });
                        }
                        for l in letters {
                            let _ = tx.send(if arrived {
                                DeviceEvent::Arrived {
                                    drive_letter: Some(l),
                                }
                            } else {
                                DeviceEvent::Removed {
                                    drive_letter: Some(l),
                                }
                            });
                        }
                    }
                });
            }
        }
        return LRESULT(0);
    }
    if msg == WM_DESTROY {
        // SAFETY: 标准窗口销毁流程
        unsafe { PostQuitMessage(0) };
        return LRESULT(0);
    }
    // SAFETY: 其余消息交给默认过程
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

/// Windows 事件驱动监听器。
///
/// 会起一个专用线程跑消息循环——窗口消息必须在创建它的线程里泵。
#[derive(Default)]
pub struct WindowsDeviceWatcher {
    thread: Option<std::thread::JoinHandle<()>>,
}

impl WindowsDeviceWatcher {
    pub fn new() -> Self {
        Self::default()
    }
}

impl DeviceWatcher for WindowsDeviceWatcher {
    fn subscribe(&mut self) -> Result<Receiver<DeviceEvent>> {
        let (tx, rx) = channel();
        let (ready_tx, ready_rx) = channel::<std::result::Result<(), String>>();

        let handle = std::thread::spawn(move || {
            SENDER.with(|s| *s.borrow_mut() = Some(tx));
            match run_message_loop(&ready_tx) {
                Ok(()) => {}
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        });
        self.thread = Some(handle);

        match ready_rx.recv() {
            Ok(Ok(())) => Ok(rx),
            Ok(Err(e)) => Err(CoreError::Terminal(
                TerminalKind::Unsupported,
                ErrorContext::new().cause(format!("设备监听启动失败：{e}")),
            )),
            Err(e) => Err(CoreError::Terminal(
                TerminalKind::Unsupported,
                ErrorContext::new().cause(format!("设备监听线程未就绪：{e}")),
            )),
        }
    }
}

fn run_message_loop(ready: &Sender<std::result::Result<(), String>>) -> std::result::Result<(), String> {
    let class_name: Vec<u16> = "SteadcopyDeviceWatcher\0".encode_utf16().collect();

    // SAFETY: 取本模块句柄用于注册窗口类
    let hinstance = unsafe { GetModuleHandleW(None) }.map_err(|e| e.to_string())?;

    let class = WNDCLASSW {
        lpfnWndProc: Some(wnd_proc),
        hInstance: windows::Win32::Foundation::HINSTANCE(hinstance.0),
        lpszClassName: PCWSTR(class_name.as_ptr()),
        ..Default::default()
    };
    // SAFETY: class 内的指针在本函数栈上存活到窗口创建完成
    let atom = unsafe { RegisterClassW(&class) };
    if atom == 0 {
        // 类可能已注册过（同进程内二次订阅），继续尝试建窗口
    }

    // 隐藏的顶层窗口：卷到达广播只发给顶层窗口，纯消息窗口收不到。
    // 尺寸 0×0 且不显示，用户看不见。
    // SAFETY: 类名以 NUL 结尾且在调用期间存活
    let hwnd = unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(class_name.as_ptr()),
            WS_OVERLAPPED,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            0,
            0,
            HWND::default(),
            windows::Win32::UI::WindowsAndMessaging::HMENU::default(),
            windows::Win32::Foundation::HINSTANCE(hinstance.0),
            None,
        )
    }
    .map_err(|e| e.to_string())?;

    let _ = ready.send(Ok(()));

    let mut msg = MSG::default();
    // SAFETY: 标准消息循环
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        let _ = DestroyWindow(hwnd);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 真机：能起得来、能拿到接收端、不崩。
    // 真插拔卡的验收在 app change 的真机清单里（自动测试无法插拔硬件）。
    #[test]
    fn scenario_preset_autorun_windows_watcher_starts() {
        let mut w = WindowsDeviceWatcher::new();
        let rx = w.subscribe().expect("监听应能启动");
        // 没有插拔动作时不该有事件——这里同时验证「噪声不当信号」：
        // 系统在此期间的 DBT_DEVNODES_CHANGED 之类通知不应变成事件。
        let got = rx.recv_timeout(std::time::Duration::from_millis(400));
        assert!(
            got.is_err(),
            "无插拔时 MUST NOT 产生事件，实际收到 {got:?}"
        );
    }
}

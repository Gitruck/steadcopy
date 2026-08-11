//! 设备到达/移除监听。**事件驱动，不轮询。**
//!
//! 规范：`openspec/changes/add-steadcopy-preset-autorun/specs/preset-autorun/spec.md`
//! → Requirement: 事件驱动的设备到达监听
//!
//! core 层只定义「到达 / 移除」两种事件与订阅接口，**不碰窗口消息**——
//! 那是平台细节，放在 `device::windows::watch`。这样 core 保持平台无关，
//! 测试也能用 `MockDeviceWatcher` 驱动完整编排（TDD 纪律 T3 允许的两类替身之一）。

use std::sync::mpsc::Receiver;

/// 设备事件。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeviceEvent {
    /// 一个卷到达。携带盘符（如 `E:`）；无盘符的卷携带 `None`，
    /// 由消费方重新枚举去认领。
    Arrived { drive_letter: Option<String> },
    Removed { drive_letter: Option<String> },
}

/// 设备监听器。
pub trait DeviceWatcher: Send {
    /// 开始监听，返回事件接收端。
    ///
    /// 实现 MUST 事件驱动，**MUST NOT** 靠定时轮询枚举卷列表。
    fn subscribe(&mut self) -> crate::error::Result<Receiver<DeviceEvent>>;
}

/// 从 Windows 的 `dbcv_unitmask` 位图解出盘符。
///
/// 位 0 = A:，位 1 = B:，以此类推。一次通知可能带多个位。
pub fn drive_letters_from_mask(mask: u32) -> Vec<String> {
    (0..26u32)
        .filter(|i| mask & (1 << i) != 0)
        .map(|i| format!("{}:", (b'A' + i as u8) as char))
        .collect()
}

/// 测试用的事件源。可以手工投递事件，不需要真插拔卡。
#[derive(Debug, Default)]
pub struct MockDeviceWatcher {
    sender: Option<std::sync::mpsc::Sender<DeviceEvent>>,
}

impl MockDeviceWatcher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 投递一个事件。未订阅时静默丢弃。
    pub fn emit(&self, event: DeviceEvent) {
        if let Some(s) = &self.sender {
            let _ = s.send(event);
        }
    }
}

impl DeviceWatcher for MockDeviceWatcher {
    fn subscribe(&mut self) -> crate::error::Result<Receiver<DeviceEvent>> {
        let (tx, rx) = std::sync::mpsc::channel();
        self.sender = Some(tx);
        Ok(rx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scenario_preset_autorun_unit_mask_decoding() {
        assert_eq!(drive_letters_from_mask(0), Vec::<String>::new());
        assert_eq!(drive_letters_from_mask(1), vec!["A:"]);
        assert_eq!(drive_letters_from_mask(1 << 4), vec!["E:"]);
        // 一次通知带多个盘（多卡槽读卡器一次插两张卡）
        assert_eq!(
            drive_letters_from_mask((1 << 4) | (1 << 5)),
            vec!["E:", "F:"]
        );
        assert_eq!(drive_letters_from_mask(1 << 25), vec!["Z:"]);
        // 超出 26 位的杂位不该产生越界盘符
        assert_eq!(drive_letters_from_mask(1 << 30), Vec::<String>::new());
    }

    #[test]
    fn scenario_preset_autorun_mock_watcher_delivers_events() {
        let mut w = MockDeviceWatcher::new();
        let rx = w.subscribe().expect("订阅");
        w.emit(DeviceEvent::Arrived {
            drive_letter: Some("E:".into()),
        });
        w.emit(DeviceEvent::Removed {
            drive_letter: Some("E:".into()),
        });
        assert_eq!(
            rx.recv().expect("收到到达"),
            DeviceEvent::Arrived {
                drive_letter: Some("E:".into())
            }
        );
        assert_eq!(
            rx.recv().expect("收到移除"),
            DeviceEvent::Removed {
                drive_letter: Some("E:".into())
            }
        );
    }
}

//! 安全弹出。
//!
//! 规范：`openspec/changes/add-steadcopy-core/specs/device-registry/spec.md`
//! → Requirement: 安全弹出
//!
//! 走系统卷管理接口，**不依赖任何外部第三方 exe**——前身那类工具常见的做法是
//! 调一个几百 KB 的第三方弹出程序，等于把「数据安全落盘」这一步的正确性
//! 外包给一个自己没读过源码的二进制。
//!
//! 弹出失败是**正常且常见**的（剪辑软件还开着素材、资源管理器停在卡上），
//! 所以失败必须给出可读原因并让程序继续跑，MUST NOT 静默吞掉。

use std::path::Path;

/// 弹出失败的原因。分类是给用户看的——「被占用」和「设备不支持」的下一步动作不同。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EjectError {
    /// 该设备上有任务在跑。这一条在到达弹出接口之前就该被拦下
    TaskRunning,
    /// 卷被其他程序占用（最常见）
    Busy(String),
    /// 本平台不支持
    Unsupported,
    /// 其他系统错误
    Failed(String),
}

impl std::fmt::Display for EjectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EjectError::TaskRunning => write!(f, "该设备上有任务正在进行，不能弹出"),
            EjectError::Busy(d) => write!(
                f,
                "设备正被其他程序占用，无法弹出（{d}）。关掉正在读这张卡的程序后再试"
            ),
            EjectError::Unsupported => write!(f, "本平台不支持安全弹出"),
            EjectError::Failed(d) => write!(f, "弹出失败：{d}"),
        }
    }
}

impl std::error::Error for EjectError {}

/// 弹出能力。抽成 trait 是为了让安全轨能用替身测编排，而不真弹卡。
pub trait Ejector: Send + Sync {
    /// 弹出指定卷。`root` 形如 `E:\` 或 `\\?\Volume{...}\`。
    fn eject(&self, root: &Path) -> Result<(), EjectError>;
}

/// 什么都不做的弹出器，非 Windows 平台与测试用。
#[derive(Debug, Default)]
pub struct UnsupportedEjector;

impl Ejector for UnsupportedEjector {
    fn eject(&self, _root: &Path) -> Result<(), EjectError> {
        Err(EjectError::Unsupported)
    }
}

/// 取本平台的弹出器。
pub fn ejector() -> Box<dyn Ejector> {
    #[cfg(windows)]
    {
        Box::new(crate::device::windows_eject::WindowsEjector)
    }
    #[cfg(not(windows))]
    {
        Box::new(UnsupportedEjector)
    }
}

/// 弹出前的准入判断。**任务进行中一律拒绝。**
///
/// 判定与执行分开：这一条能在安全轨里被测，不需要真的有一张卡。
pub fn can_eject(device_id: &str, running_device_ids: &[String]) -> Result<(), EjectError> {
    if running_device_ids.iter().any(|d| d == device_id) {
        return Err(EjectError::TaskRunning);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // spec: → Scenario: 任务进行中拒绝弹出
    #[test]
    fn scenario_device_registry_eject_refused_while_task_running() {
        let running = vec!["vol:abc".to_string()];
        assert_eq!(
            can_eject("vol:abc", &running),
            Err(EjectError::TaskRunning),
            "正在拷的设备不能弹"
        );
        assert_eq!(can_eject("vol:xyz", &running), Ok(()), "别的设备不受影响");
        assert_eq!(can_eject("vol:abc", &[]), Ok(()), "没任务在跑就可以弹");
    }

    // spec: → Scenario: 被占用时的可读失败
    #[test]
    fn scenario_device_registry_eject_failure_is_readable() {
        // 每一种失败都要能对用户说人话，且要能指出下一步做什么
        let busy = EjectError::Busy("卷被锁定".into());
        let msg = busy.to_string();
        assert!(msg.contains("占用"), "{msg}");
        assert!(msg.contains("再试"), "要给下一步动作：{msg}");

        for e in [
            EjectError::TaskRunning,
            EjectError::Unsupported,
            EjectError::Failed("句柄无效".into()),
        ] {
            assert!(!e.to_string().is_empty(), "{e:?} 必须有可读描述");
        }

        // 不支持的平台明确报「不支持」，不假装成功
        assert_eq!(
            UnsupportedEjector.eject(Path::new("E:\\")),
            Err(EjectError::Unsupported)
        );
    }
}

//! 自动、共享与独立通道模式的配置解析及选择。
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Shared,
    Independent,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    Luoye,
    Qmodem,
    Glinet,
    Independent,
}
pub fn with_modem_cache(
    requested: Option<bool>,
    qmodem: bool,
    glinet: bool,
    modem_cache: bool,
) -> Backend {
    // 已安装的原 modem 管理器拥有事务通道，缓存失败不能触发另一套串口驱动。
    if modem_cache {
        Backend::Luoye
    } else {
        backend(requested, qmodem, glinet)
    }
}
pub fn backend(requested_standalone: Option<bool>, qmodem: bool, glinet: bool) -> Backend {
    if requested_standalone == Some(true) {
        Backend::Independent
    } else if qmodem {
        Backend::Qmodem
    } else if glinet {
        Backend::Glinet
    } else if requested_standalone == Some(false) {
        Backend::Qmodem
    } else {
        Backend::Independent
    }
}
impl Mode {
    pub fn name(self) -> &'static str {
        match self {
            Self::Shared => "shared",
            Self::Independent => "independent",
        }
    }
}
pub fn qmodem_installed(root: &Path) -> bool {
    [
        "usr/share/qmodem/modem_util.sh",
        "usr/share/qmodem/modem_support.json",
    ]
    .iter()
    .all(|name| std::fs::symlink_metadata(root.join(name)).is_ok_and(|m| m.is_file()))
}
pub fn automatic(qmodem_present: bool) -> Mode {
    if qmodem_present {
        Mode::Shared
    } else {
        Mode::Independent
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn auto_and_explicit_modes_keep_shared_managers_ahead_of_independent_ports() {
        use super::{Backend, backend};
        for qmodem in [false, true] {
            for glinet in [false, true] {
                let detected = if qmodem {
                    Backend::Qmodem
                } else if glinet {
                    Backend::Glinet
                } else {
                    Backend::Independent
                };
                assert_eq!(backend(None, qmodem, glinet), detected);
                assert_eq!(backend(Some(true), qmodem, glinet), Backend::Independent);
                assert_eq!(
                    backend(Some(false), qmodem, glinet),
                    if glinet && !qmodem {
                        Backend::Glinet
                    } else {
                        Backend::Qmodem
                    }
                );
            }
        }
    }
    #[test]
    fn manager_presence_selects_transport_not_permission_to_take_ports() {
        assert_eq!(super::automatic(true), super::Mode::Shared);
        assert_eq!(super::automatic(false), super::Mode::Independent);
    }
}

//! 自动、共享与独立通道模式的配置解析及选择。
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Shared,
    Independent,
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
    fn manager_presence_selects_transport_not_permission_to_take_ports() {
        assert_eq!(super::automatic(true), super::Mode::Shared);
        assert_eq!(super::automatic(false), super::Mode::Independent);
    }
}

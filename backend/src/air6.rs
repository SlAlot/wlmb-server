//! 软银 Air 6 的 MIPC 拨号和蜂窝状态适配。
pub const PROVIDER: &str = "air6-mipc";

#[cfg(unix)]
mod native;
#[cfg(unix)]
pub use native::{Air6, Operation, detected, netifd, status};

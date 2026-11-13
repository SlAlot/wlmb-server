//! SoftBank Air 6 integration.
pub const PROVIDER: &str = "air6-mipc";

#[cfg(unix)]
mod native;
#[cfg(unix)]
pub use native::{Air6, Operation, detected, netifd, status};

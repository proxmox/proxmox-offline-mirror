use std::fmt::Display;

use proxmox_time::{epoch_i64, epoch_to_rfc3339_utc};

#[derive(Debug, Clone, Copy)]
pub struct Snapshot(i64);

impl Snapshot {
    pub fn now() -> Self {
        Self(epoch_i64())
    }
}

impl Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = epoch_to_rfc3339_utc(self.0).map_err(|_| std::fmt::Error)?;
        f.write_str(&formatted)
    }
}

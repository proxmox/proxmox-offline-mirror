use std::fmt::Debug;

use proxmox_apt::repositories::APTRepository;

#[derive(Debug)]
pub struct MirrorConfig {
    pub repository: APTRepository,
    pub architectures: Vec<String>,
    pub path: String,
    pub key: Vec<u8>,
}

use std::path::Path;

use anyhow::{bail, Error};
use lazy_static::lazy_static;
use serde::{Deserialize, Serialize};

use proxmox_apt::repositories::APTRepository;
use proxmox_schema::{api, ApiType, Schema, Updater};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};

use crate::{convert_repo_line, pool::Pool, types::MIRROR_ID_SCHEMA};

#[api(
    properties: {
        id: {
            schema: MIRROR_ID_SCHEMA,
        },
        repository: {
            type: String,
            description: "Single repository definition in sources.list format.",
        },
        architectures: {
            type: Array,
            description: "List of architectures to mirror",
            items: {
                type: String,
                description: "Architecture specifier.",
            },
        },
        "pool-dir": {
            type: String,
            description: "Path to pool directory storing checksum files.",
        },
        "base-dir": {
            type: String,
            description: "Path to directory storing repository snapshot files (must be on same FS as `pool-dir`).",
        },
        "key-path": {
            type: String,
            description: "Path to signing key of `repository`",
        },
        verify: {
            type: bool,
            description: "Whether to verify existing files stored in pool (IO-intensive).",
        },
        sync: {
            type: bool,
            description: "Whether to write pool updates with fsync flag.",
        },
    }
)]
#[derive(Clone, Debug, Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// Configuration file for mirrored repositories.
pub struct MirrorConfig {
    #[updater(skip)]
    pub id: String,
    pub repository: String,
    pub architectures: Vec<String>,
    pub pool_dir: String,
    pub base_dir: String,
    pub key_path: String,
    pub verify: bool,
    pub sync: bool,
}

impl TryInto<Pool> for &MirrorConfig {
    type Error = Error;

    fn try_into(self) -> Result<Pool, Self::Error> {
        Pool::open(Path::new(&self.base_dir), Path::new(&self.pool_dir))
    }
}

#[api(
    properties: {
        id: {
            schema: MIRROR_ID_SCHEMA,
        },
        mountpoint: {
            type: String,
            description: "Path where sync target is mounted."
        },
        verify: {
            type: bool,
            description: "Whether to verify existing files stored in pool (IO-intensive).",
        },
        sync: {
            type: bool,
            description: "Whether to write pool updates with fsync flag.",
        },
        mirrors: {
            type: Array,
            description: "List of mirror IDs this sync target should contain.",
            items: {
                schema: MIRROR_ID_SCHEMA,
            },
        },
    }
)]
#[derive(Debug, Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// Configuration file for mirrored repositories.
pub struct MediaConfig {
    #[updater(skip)]
    pub id: String,
    pub mountpoint: String,
    pub mirrors: Vec<String>,
    pub verify: bool,
    pub sync: bool,
}

lazy_static! {
    pub static ref CONFIG: SectionConfig = init();
}

fn init() -> SectionConfig {
    let mut config = SectionConfig::new(&MIRROR_ID_SCHEMA);

    let mirror_schema = match MirrorConfig::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };
    let mirror_plugin = SectionConfigPlugin::new(
        "mirror".to_string(),
        Some(String::from("id")),
        mirror_schema,
    );
    config.register_plugin(mirror_plugin);

    let media_schema = match MediaConfig::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };
    let media_plugin =
        SectionConfigPlugin::new("medium".to_string(), Some(String::from("id")), media_schema);
    config.register_plugin(media_plugin);

    config
}

pub struct ConfigLockGuard(std::fs::File);

/// Get exclusive lock
pub fn lock_config(path: &str) -> Result<ConfigLockGuard, Error> {
    let path = Path::new(path);

    let (mut path, file) = match (path.parent(), path.file_name()) {
        (Some(parent), Some(file)) => (parent.to_path_buf(), file.to_string_lossy()),
        _ => bail!("Unable to derive lock file name for {path:?}"),
    };
    path.push(format!(".{file}.lock"));

    let file = proxmox_sys::fs::open_file_locked(
        &path,
        std::time::Duration::new(10, 0),
        true,
        CreateOptions::default(),
    )?;
    Ok(ConfigLockGuard(file))
}

pub fn config(path: &str) -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(path)?.unwrap_or_else(|| "".to_string());

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(path, &content)?;
    Ok((data, digest))
}

pub fn save_config(path: &str, data: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(path, data)?;
    replace_file(path, raw.as_bytes(), CreateOptions::default(), true)
}

pub struct ParsedMirrorConfig {
    pub repository: APTRepository,
    pub architectures: Vec<String>,
    pub pool: Pool,
    pub key: Vec<u8>,
    pub verify: bool,
    pub sync: bool,
}

impl TryInto<ParsedMirrorConfig> for MirrorConfig {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ParsedMirrorConfig, Self::Error> {
        let pool = (&self).try_into()?;

        let repository = convert_repo_line(self.repository.clone())?;

        let key = file_get_contents(Path::new(&self.key_path))?;

        Ok(ParsedMirrorConfig {
            repository,
            architectures: self.architectures,
            pool,
            key,
            verify: self.verify,
            sync: self.sync,
        })
    }
}

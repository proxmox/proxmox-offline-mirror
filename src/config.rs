use std::path::Path;

use anyhow::{bail, Error};
use lazy_static::lazy_static;
use proxmox_subscription::{sign::ServerBlob, SubscriptionInfo};
use serde::{Deserialize, Serialize};

use proxmox_schema::{api, ApiType, Schema, Updater};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};
use proxmox_sys::fs::{replace_file, CreateOptions};

use crate::types::{
    ProductType, MEDIA_ID_SCHEMA, MIRROR_ID_SCHEMA, PROXMOX_SERVER_ID_SCHEMA,
    PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
};

#[api(
    properties: {
        id: {
            schema: MIRROR_ID_SCHEMA,
        },
        repository: {
            type: String,
        },
        architectures: {
            type: Array,
            items: {
                type: String,
                description: "Architecture specifier.",
            },
        },
        "dir": {
            type: String,
        },
        "key-path": {
            type: String,
        },
        verify: {
            type: bool,
        },
        sync: {
            type: bool,
        },
    }
)]
#[derive(Clone, Debug, Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// Configuration entry for a mirrored repository.
pub struct MirrorConfig {
    #[updater(skip)]
    /// Identifier for this entry.
    pub id: String,
    /// Single repository definition in sources.list format.
    pub repository: String,
    /// List of architectures that should be mirrored.
    pub architectures: Vec<String>,
    /// Path to directory containg mirrored repository.
    pub dir: String,
    /// Path to public key file for verifying repository integrity.
    pub key_path: String,
    /// Whether to verify existing files or assume they are valid (IO-intensive).
    pub verify: bool,
    /// Whether to write new files using FSYNC.
    pub sync: bool,
    /// Use subscription key to access (required for Proxmox Enterprise repositories).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_subscription: Option<ProductType>,
}

#[api(
    properties: {
        id: {
            schema: MEDIA_ID_SCHEMA,
        },
        mountpoint: {
            type: String,
        },
        verify: {
            type: bool,
        },
        sync: {
            type: bool,
        },
        mirrors: {
            type: Array,
            items: {
                schema: MIRROR_ID_SCHEMA,
            },
        },
    }
)]
#[derive(Debug, Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// Configuration entry for an external medium.
pub struct MediaConfig {
    #[updater(skip)]
    /// Identifier for this entry.
    pub id: String,
    /// Mountpoint where medium is available on mirroring system.
    pub mountpoint: String,
    /// List of [MirrorConfig] IDs which should be synced to medium.
    pub mirrors: Vec<String>,
    /// Whether to verify existing files or assume they are valid (IO-intensive).
    pub verify: bool,
    /// Whether to write new files using FSYNC.
    pub sync: bool,
}

#[api(
    properties: {
        key: {
            schema: PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
        },
        "server-id": {
            schema: PROXMOX_SERVER_ID_SCHEMA,
        },
        description: {
            type: String,
            optional: true,
        },
        info: {
            type: String,
            description: "base64 encoded subscription info - update with 'refresh' command.",
            optional: true,
        },
    },
)]
#[derive(Clone, Debug, Serialize, Deserialize, Updater)]
#[serde(rename_all = "kebab-case")]
/// Subscription key used for accessing enterprise repositories and for offline subscription activation/renewal.
pub struct SubscriptionKey {
    /// Subscription key
    #[updater(skip)]
    pub key: String,
    /// Server ID for this subscription key
    pub server_id: String,
    /// Description, e.g. which system this key is deployed on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Last Subscription Key state
    #[serde(skip_serializing_if = "Option::is_none")]
    #[updater(skip)]
    pub info: Option<String>,
}

impl Into<ServerBlob> for SubscriptionKey {
    fn into(self) -> ServerBlob {
        ServerBlob {
            key: self.key,
            serverid: self.server_id,
        }
    }
}

impl SubscriptionKey {
    pub fn product(&self) -> ProductType {
        match &self.key[..3] {
            "pve" => ProductType::Pve,
            "pmg" => ProductType::Pmg,
            "pbs" => ProductType::Pbs,
            "pom" => ProductType::Pom, // TODO replace with actual key prefix
            _ => unimplemented!(),
        }
    }

    pub fn info(&self) -> Result<Option<SubscriptionInfo>, Error> {
        match self.info.as_ref() {
            Some(info) => {
                let info = base64::decode(info)?;
                let info = serde_json::from_slice(&info)?;
                Ok(Some(info))
            }
            None => Ok(None),
        }
    }
}

lazy_static! {
    static ref CONFIG: SectionConfig = init();
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

    let key_schema = match SubscriptionKey::API_SCHEMA {
        Schema::Object(ref obj_schema) => obj_schema,
        _ => unreachable!(),
    };
    let key_plugin = SectionConfigPlugin::new(
        "subscription".to_string(),
        Some(String::from("key")),
        key_schema,
    );
    config.register_plugin(key_plugin);

    config
}

/// Lock guard for guarding modifications of config file.
///
/// Obtained via [lock_config], should only be dropped once config file should no longer be locked.
pub struct ConfigLockGuard(std::fs::File);

/// Get exclusive lock for config file (in order to make or protect against modifications).
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

/// Read config
pub fn config(path: &str) -> Result<(SectionConfigData, [u8; 32]), Error> {
    let content =
        proxmox_sys::fs::file_read_optional_string(path)?.unwrap_or_else(|| "".to_string());

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(path, &content)?;
    Ok((data, digest))
}

/// Write config (and verify data matches schema!)
pub fn save_config(path: &str, data: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(path, data)?;
    replace_file(path, raw.as_bytes(), CreateOptions::default(), true)
}

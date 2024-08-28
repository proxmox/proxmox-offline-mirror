use std::path::Path;
use std::sync::LazyLock;

use anyhow::{bail, Error};
use proxmox_subscription::{sign::ServerBlob, SubscriptionInfo};
use serde::{Deserialize, Serialize};

use proxmox_schema::{api, ApiStringFormat, ApiType, Updater};
use proxmox_section_config::{SectionConfig, SectionConfigData, SectionConfigPlugin};
use proxmox_subscription::ProductType;
use proxmox_sys::fs::{replace_file, CreateOptions};

use crate::types::{
    MEDIA_ID_SCHEMA, MIRROR_ID_SCHEMA, PROXMOX_SERVER_ID_SCHEMA, PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
};

/// Skip Configuration
#[api(
    properties: {
        "skip-sections": {
            type: Array,
            optional: true,
            items: {
                type: String,
                description: "Section name",
            },
        },
        "skip-packages": {
            type: Array,
            optional: true,
            items: {
                type: String,
                description: "Package name",
            },
        },
    },
)]
#[derive(Default, Serialize, Deserialize, Updater, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
pub struct SkipConfig {
    /// Sections which should be skipped
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_sections: Option<Vec<String>>,
    /// Packages which should be skipped, supports globbing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub skip_packages: Option<Vec<String>>,
}

#[api(
    properties: {
        "allow-sha1": {
            type: bool,
            default: false,
            optional: true,
        },
        "min-dsa-key-size": {
            type: u64,
            optional: true,
        },
        "min-rsa-key-size": {
            type: u64,
            optional: true,
        },
    },
)]
#[derive(Default, Serialize, Deserialize, Updater, Clone, Debug)]
#[serde(rename_all = "kebab-case")]
/// Weak Cryptography Configuration
pub struct WeakCryptoConfig {
    /// Whether to allow SHA-1 based signatures
    #[serde(default)]
    pub allow_sha1: bool,
    /// Whether to lower the key size cutoff for DSA-based signatures
    #[serde(default)]
    pub min_dsa_key_size: Option<u64>,
    /// Whether to lower the key size cutoff for RSA-based signatures
    #[serde(default)]
    pub min_rsa_key_size: Option<u64>,
}

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
        "base-dir": {
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
        "ignore-errors": {
            type: bool,
            optional: true,
            default: false,
        },
        "skip": {
            type: SkipConfig,
        },
        "weak-crypto": {
            type: String,
            optional: true,
            format: &ApiStringFormat::PropertyString(&WeakCryptoConfig::API_SCHEMA),
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
    /// Path to directory containg mirrored repository pool. Can be shared by multiple mirrors.
    pub base_dir: String,
    /// Path to public key file for verifying repository integrity.
    pub key_path: String,
    /// Whether to verify existing files or assume they are valid (IO-intensive).
    pub verify: bool,
    /// Whether to write new files using FSYNC.
    pub sync: bool,
    /// Use subscription key to access (required for Proxmox Enterprise repositories).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_subscription: Option<ProductType>,
    /// Whether to downgrade download errors to warnings
    #[serde(default)]
    pub ignore_errors: bool,
    /// Skip package files using these criteria
    #[serde(default, flatten)]
    pub skip: SkipConfig,
    /// Whether to allow using weak cryptography algorithms or parameters, deviating from the default policy.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weak_crypto: Option<String>,
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

impl From<SubscriptionKey> for ServerBlob {
    fn from(key: SubscriptionKey) -> Self {
        Self {
            key: key.key,
            serverid: key.server_id,
        }
    }
}

impl SubscriptionKey {
    pub fn product(&self) -> ProductType {
        match &self.key[..3] {
            "pve" => ProductType::Pve,
            "pmg" => ProductType::Pmg,
            "pbs" => ProductType::Pbs,
            "pom" => ProductType::Pom,
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

pub static CONFIG: LazyLock<SectionConfig> = LazyLock::new(init);

fn init() -> SectionConfig {
    let mut config = SectionConfig::new(&MIRROR_ID_SCHEMA);

    let mirror_plugin = SectionConfigPlugin::new(
        "mirror".to_string(),
        Some(String::from("id")),
        const { MirrorConfig::API_SCHEMA.unwrap_any_object_schema() },
    );
    config.register_plugin(mirror_plugin);

    let media_plugin = SectionConfigPlugin::new(
        "medium".to_string(),
        Some(String::from("id")),
        const { MediaConfig::API_SCHEMA.unwrap_any_object_schema() },
    );
    config.register_plugin(media_plugin);

    let key_plugin = SectionConfigPlugin::new(
        "subscription".to_string(),
        Some(String::from("key")),
        const { SubscriptionKey::API_SCHEMA.unwrap_any_object_schema() },
    );
    config.register_plugin(key_plugin);

    config
}

/// Lock guard for guarding modifications of config file.
///
/// Obtained via [lock_config], should only be dropped once config file should no longer be locked.
#[allow(dead_code)]
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
    let content = proxmox_sys::fs::file_read_optional_string(path)?.unwrap_or_default();

    let digest = openssl::sha::sha256(content.as_bytes());
    let data = CONFIG.parse(path, &content)?;
    Ok((data, digest))
}

/// Write config (and verify data matches schema!)
pub fn save_config(path: &str, data: &SectionConfigData) -> Result<(), Error> {
    let raw = CONFIG.write(path, data)?;
    replace_file(path, raw.as_bytes(), CreateOptions::default(), true)
}

use anyhow::{bail, format_err, Error};

use serde::Serialize;
use serde_json::Value;
use std::convert::TryFrom;

use proxmox_offline_mirror::{
    config::{SubscriptionKey, SubscriptionKeyUpdater},
    subscription::{extract_mirror_key, refresh},
    types::{ProductType, PROXMOX_SUBSCRIPTION_KEY_SCHEMA},
};
use proxmox_subscription::{files::DEFAULT_SIGNING_KEY, SubscriptionStatus};
use proxmox_sys::fs::file_get_contents;
use proxmox_time::epoch_to_rfc3339_utc;

use proxmox_router::cli::{
    default_table_format_options, format_and_print_result_full, get_output_format, CliCommand,
    CliCommandMap, ColumnConfig, CommandLineInterface, OUTPUT_FORMAT,
};
use proxmox_schema::{api, param_bail, ApiType, ArraySchema, ReturnType};

use super::DEFAULT_CONFIG_PATH;

#[api]
#[derive(Default, Serialize)]
#[serde(rename_all = "kebab-case")]
/// `[SubscriptionKey]` with contained in info cross-checked and decoded.
struct DecodedSubscriptionKey {
    /// Subscription key
    pub key: String,
    /// Server ID for this subscription key
    pub server_id: String,
    /// Description, e.g. which system this key is deployed on
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Last Subscription Key state
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<SubscriptionStatus>,
    /// timestamp of the last check done
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checktime: Option<String>,
    /// a more human readable status message
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    /// human readable productname of the set subscription
    #[serde(skip_serializing_if = "Option::is_none")]
    pub productname: Option<String>,
    /// register date of the set subscription
    #[serde(skip_serializing_if = "Option::is_none")]
    pub regdate: Option<String>,
    /// next due date of the set subscription
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nextduedate: Option<String>,
    /// Signature status
    pub signed: bool,
}

impl TryFrom<SubscriptionKey> for DecodedSubscriptionKey {
    type Error = Error;

    fn try_from(value: SubscriptionKey) -> Result<Self, Self::Error> {
        let info = value.info()?;

        if let Some(mut info) = info {
            if let Some(key) = info.key.as_ref() {
                if key != &value.key {
                    bail!(
                        "key '{}' doesn't match subscription info key '{}'",
                        value.key,
                        key
                    )
                }
            }
            if let Some(serverid) = info.serverid.as_ref() {
                if serverid != &value.server_id {
                    bail!(
                        "server ID '{}' doesn't match subscription info key '{}'",
                        value.server_id,
                        serverid
                    )
                }
            }

            let signed = info.is_signed();
            if signed {
                info.check_signature(&[DEFAULT_SIGNING_KEY]);
            }

            let checktime = info.checktime.map(epoch_to_rfc3339_utc).transpose()?;

            Ok(DecodedSubscriptionKey {
                key: value.key,
                server_id: value.server_id,
                description: value.description,
                status: Some(info.status),
                checktime,
                message: info.message,
                regdate: info.regdate,
                nextduedate: info.nextduedate,
                productname: info.productname,
                signed,
            })
        } else {
            Ok(DecodedSubscriptionKey {
                key: value.key,
                server_id: value.server_id,
                description: value.description,
                signed: false,
                ..Default::default()
            })
        }
    }
}

pub const LIST_KEYS_RETURN_TYPE: ReturnType = ReturnType {
    optional: false,
    schema: &ArraySchema::new(
        "Returns the list of subscription keys.",
        &DecodedSubscriptionKey::API_SCHEMA,
    )
    .schema(),
};

pub const SHOW_KEY_RETURN_TYPE: ReturnType = ReturnType {
    schema: &DecodedSubscriptionKey::API_SCHEMA,
    optional: true,
};

fn public_key() -> Result<openssl::pkey::PKey<openssl::pkey::Public>, Error> {
    openssl::pkey::PKey::public_key_from_pem(&file_get_contents(DEFAULT_SIGNING_KEY)?)
        .map_err(Error::from)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// List subscription keys and their status
async fn list_keys(config: Option<String>, param: Value) -> Result<(), Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: Vec<SubscriptionKey> = config.convert_to_typed_array("subscription")?;
    let decoded: Vec<DecodedSubscriptionKey> =
        config.into_iter().fold(Vec::new(), |mut values, key| {
            match key.clone().try_into() {
                Ok(decoded) => values.push(decoded),
                Err(err) => {
                    values.push(DecodedSubscriptionKey {
                        key: key.key,
                        server_id: key.server_id,
                        description: key.description,
                        message: Some(format!("Failed to decode info - {err}")),
                        ..Default::default()
                    });
                }
            };
            values
        });
    let output_format = get_output_format(&param);
    let options = default_table_format_options()
        .column(ColumnConfig::new("key").header("Subscription Key"))
        .column(ColumnConfig::new("server-id").header("Server ID"))
        .column(ColumnConfig::new("description"))
        .column(ColumnConfig::new("status").header("Status"))
        .column(ColumnConfig::new("message").header("Message"))
        .column(ColumnConfig::new("checktime").header("Last Check"))
        .column(ColumnConfig::new("nextduedate").header("Next Due"))
        .column(ColumnConfig::new("signed").header("Signed"));
    format_and_print_result_full(
        &mut serde_json::json!(decoded),
        &LIST_KEYS_RETURN_TYPE,
        &output_format,
        &options,
    );

    Ok(())
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            key: {
                schema: PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
            },
        }
    },
 )]
/// Add offline mirror key
async fn add_mirror_key(config: Option<String>, key: String, _param: Value) -> Result<(), Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let _lock = proxmox_offline_mirror::config::lock_config(&config)?;

    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;

    if section_config.sections.get(&key).is_some() {
        param_bail!(
            "key",
            "key entry for '{}' already exists - did you mean to update or refresh?",
            key
        );
    }

    let server_id = proxmox_subscription::get_hardware_address()?;
    let mut data = SubscriptionKey {
        key,
        server_id,
        description: None,
        info: None,
    };

    if data.product() != ProductType::Pom {
        param_bail!(
            "key",
            format_err!(
                "Only Proxmox Offline Mirror keys can be added with 'add-mirror-key' command."
            )
        );
    }

    let mut refreshed =
        proxmox_offline_mirror::subscription::refresh(data.clone(), vec![], public_key()?).await?;

    if let Some(info) = refreshed.pop() {
        eprintln!(
            "Refreshed subscription info - status: {}, message: {}",
            info.status,
            info.message.as_ref().unwrap_or(&"-".to_string())
        );

        if info.key.as_ref() == Some(&data.key) {
            data.info = Some(base64::encode(serde_json::to_vec(&info)?));
        } else {
            bail!("Server returned subscription info for wrong key.");
        }
    }

    section_config.set_data(&data.key, "subscription", &data)?;
    proxmox_offline_mirror::config::save_config(&config, &section_config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            data: {
                type: SubscriptionKey,
                flatten: true,
            },
            refresh: {
                type: bool,
                optional: true,
                default: true,
                description: "Whether to refresh the subscription info upon adding.",
            },
        }
    },
 )]
/// List subscription keys and their status
async fn add_key(
    config: Option<String>,
    mut data: SubscriptionKey,
    refresh: bool,
    _param: Value,
) -> Result<(), Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let _lock = proxmox_offline_mirror::config::lock_config(&config)?;

    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;

    if section_config.sections.get(&data.key).is_some() {
        param_bail!(
            "key",
            "key entry for '{}' already exists - did you mean to update or refresh?",
            data.key
        );
    }

    if data.product() == ProductType::Pom {
        param_bail!(
            "key",
            format_err!("Proxmox Offline Mirror keys must be added with 'add-mirror-key' command.")
        );
    }

    if refresh {
        let mirror_key =
            extract_mirror_key(&section_config.convert_to_typed_array("subscription")?)?;

        let mut refreshed = proxmox_offline_mirror::subscription::refresh(
            mirror_key,
            vec![data.clone()],
            public_key()?,
        )
        .await?;

        if let Some(info) = refreshed.pop() {
            if info.key.as_ref() == Some(&data.key) {
                data.info = Some(base64::encode(serde_json::to_vec(&info)?));
            } else {
                bail!("Server returned subscription info for wrong key.");
            }
        }
    }

    section_config.set_data(&data.key, "subscription", &data)?;
    proxmox_offline_mirror::config::save_config(&config, &section_config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            key: {
                schema: PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Show (decoded) subscription config entry.
pub fn show_key(config: Option<String>, key: String, param: Value) -> Result<(), Error> {
    let config_file = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;

    let data: SubscriptionKey = config.lookup("subscription", &key)?;
    let decoded: DecodedSubscriptionKey = data.try_into()?;

    let output_format = get_output_format(&param);

    format_and_print_result_full(
        &mut serde_json::json!(decoded),
        &SHOW_KEY_RETURN_TYPE,
        &output_format,
        &default_table_format_options(),
    );

    Ok(())
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            key: {
                schema: PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
            },
            update: {
                type: SubscriptionKeyUpdater,
                flatten: true,
            },
        },
    },
)]
/// Update subscription config entry.
pub fn update_key(
    update: SubscriptionKeyUpdater,
    config: Option<String>,
    key: String,
) -> Result<(), Error> {
    let config_file = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;

    let mut data: SubscriptionKey = config.lookup("subscription", &key)?;

    if let Some(server_id) = update.server_id {
        data.server_id = server_id;
    }
    if let Some(description) = update.description {
        data.description = Some(description);
    }

    config.set_data(&key, "subscription", &data)?;
    proxmox_offline_mirror::config::save_config(&config_file, &config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            key: {
                schema: PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
                optional: true,
            },
        },
    },
)]
/// Refresh subscription key status.
pub async fn refresh_keys(config: Option<String>, key: Option<String>) -> Result<(), Error> {
    let config_file = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;

    let keys: Vec<SubscriptionKey> = config.convert_to_typed_array("subscription")?;
    let mirror_key = extract_mirror_key(&keys)?;

    let refreshed = if let Some(key) = key {
        match keys.iter().find(|k| k.key == key) {
            Some(key) => refresh(mirror_key, vec![key.to_owned()], public_key()?).await?,
            None => bail!("Subscription key '{key}' not configured."),
        }
    } else {
        refresh(mirror_key, keys, public_key()?).await?
    };

    for info in refreshed {
        match info.clone().key {
            Some(key) => {
                eprintln!(
                    "Refreshed subscription info - key: {}, status: {}, message: {}",
                    &key,
                    info.status,
                    info.message.as_ref().unwrap_or(&"-".to_string())
                );
                let key = key.clone();
                let mut data: SubscriptionKey = config.lookup("subscription", &key)?;
                data.info = Some(base64::encode(serde_json::to_vec(&info)?));
                config.set_data(&key, "subscription", data)?;
            }
            None => bail!("Server returned subscription key which was not queried!"),
        }
    }

    proxmox_offline_mirror::config::save_config(&config_file, &config)?;

    Ok(())
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            key: {
                schema: PROXMOX_SUBSCRIPTION_KEY_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Remove subscription key config entry.
async fn remove_key(config: Option<String>, key: String, _param: Value) -> Result<Value, Error> {
    let config_file = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;
    match section_config.lookup::<SubscriptionKey>("subscription", &key) {
        Ok(_config) => {
            section_config.sections.remove(&key);
        }
        _ => {
            param_bail!("key", "config section '{}' does not exist!", key);
        }
    }

    proxmox_offline_mirror::config::save_config(&config_file, &section_config)?;

    Ok(Value::Null)
}

pub fn key_commands() -> CommandLineInterface {
    CliCommandMap::new()
        .insert(
            "add",
            CliCommand::new(&API_METHOD_ADD_KEY).arg_param(&["key", "server-id"]),
        )
        .insert(
            "add-mirror-key",
            CliCommand::new(&API_METHOD_ADD_MIRROR_KEY).arg_param(&["key"]),
        )
        .insert(
            "show",
            CliCommand::new(&API_METHOD_SHOW_KEY).arg_param(&["key"]),
        )
        .insert(
            "update",
            CliCommand::new(&API_METHOD_UPDATE_KEY).arg_param(&["key"]),
        )
        .insert("refresh", CliCommand::new(&API_METHOD_REFRESH_KEYS))
        .insert(
            "remove",
            CliCommand::new(&API_METHOD_REMOVE_KEY).arg_param(&["key"]),
        )
        .insert("list", CliCommand::new(&API_METHOD_LIST_KEYS))
        .into()
}

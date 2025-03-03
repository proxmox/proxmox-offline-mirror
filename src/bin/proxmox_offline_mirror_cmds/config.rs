use std::{env, fs::remove_dir_all, path::Path};

use anyhow::{Error, bail};
use serde_json::Value;

use proxmox_router::cli::{
    CliCommand, CliCommandMap, ColumnConfig, CommandLineInterface, OUTPUT_FORMAT,
    default_table_format_options, format_and_print_result_full, get_output_format,
};
use proxmox_schema::{ApiType, ArraySchema, ReturnType, api, param_bail};

use proxmox_offline_mirror::{
    config::{MediaConfig, MediaConfigUpdater, MirrorConfig, MirrorConfigUpdater},
    mirror,
    types::{MEDIA_ID_SCHEMA, MIRROR_ID_SCHEMA},
};

pub fn get_config_path() -> String {
    env::var("PROXMOX_OFFLINE_MIRROR_CONFIG")
        .unwrap_or_else(|_| "/etc/proxmox-offline-mirror.cfg".to_string())
}

pub const LIST_MIRRORS_RETURN_TYPE: ReturnType = ReturnType {
    optional: false,
    schema: &ArraySchema::new("Returns the list of mirrors.", &MirrorConfig::API_SCHEMA).schema(),
};

pub const SHOW_MIRROR_RETURN_TYPE: ReturnType = ReturnType {
    schema: &MirrorConfig::API_SCHEMA,
    optional: true,
};

pub const LIST_MEDIA_RETURN_TYPE: ReturnType = ReturnType {
    optional: false,
    schema: &ArraySchema::new("Returns the list of mirrors.", &MediaConfig::API_SCHEMA).schema(),
};

pub const SHOW_MEDIUM_RETURN_TYPE: ReturnType = ReturnType {
    schema: &MediaConfig::API_SCHEMA,
    optional: true,
};

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
/// List configured mirrors
async fn list_mirror(config: Option<String>, param: Value) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: Vec<MirrorConfig> = config.convert_to_typed_array("mirror")?;

    let output_format = get_output_format(&param);
    let options = default_table_format_options()
        .column(ColumnConfig::new("id").header("ID"))
        .column(ColumnConfig::new("repository"))
        .column(ColumnConfig::new("base-dir"))
        .column(ColumnConfig::new("verify"))
        .column(ColumnConfig::new("sync"));

    format_and_print_result_full(
        &mut serde_json::json!(config),
        &LIST_MIRRORS_RETURN_TYPE,
        &output_format,
        &options,
    );

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Show full mirror config
async fn show_mirror(config: Option<String>, id: String, param: Value) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let mut config = config.lookup_json("mirror", &id)?;

    let output_format = get_output_format(&param);
    format_and_print_result_full(
        &mut config,
        &SHOW_MIRROR_RETURN_TYPE,
        &output_format,
        &default_table_format_options(),
    );
    Ok(Value::Null)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            data: {
                type: MirrorConfig,
                flatten: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Create new mirror config entry.
async fn add_mirror(
    config: Option<String>,
    data: MirrorConfig,
    _param: Value,
) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config)?;

    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;

    if section_config.sections.contains_key(&data.id) {
        param_bail!("name", "mirror config entry '{}' already exists.", data.id);
    }

    mirror::init(&data)?;

    section_config.set_data(&data.id, "mirror", &data)?;
    proxmox_offline_mirror::config::save_config(&config, &section_config)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
            "remove-data": {
                type: bool,
                description: "Remove mirror data as well.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Remove mirror config entry.
async fn remove_mirror(
    config: Option<String>,
    id: String,
    remove_data: bool,
    _param: Value,
) -> Result<Value, Error> {
    let config_file = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    // TODO (optionally?) remove media entries?
    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;
    match section_config.lookup::<MirrorConfig>("mirror", &id) {
        Ok(config) => {
            if remove_data {
                mirror::destroy(&config)?;
            }

            section_config.sections.remove(&id);
        }
        _ => {
            param_bail!("id", "mirror config entry '{}' does not exist!", id);
        }
    }

    proxmox_offline_mirror::config::save_config(&config_file, &section_config)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
            update: {
                type: MirrorConfigUpdater,
                flatten: true,
            },
        },
    },
)]
/// Update mirror config entry.
pub fn update_mirror(
    update: MirrorConfigUpdater,
    config: Option<String>,
    id: String,
) -> Result<(), Error> {
    let config_file = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;

    let mut data: MirrorConfig = config.lookup("mirror", &id)?;

    if let Some(key_path) = update.key_path {
        data.key_path = key_path
    }
    if let Some(repository) = update.repository {
        data.repository = repository
    }
    if let Some(base_dir) = update.base_dir {
        data.base_dir = base_dir
    }
    if let Some(architectures) = update.architectures {
        data.architectures = architectures
    }
    if let Some(sync) = update.sync {
        data.sync = sync
    }
    if let Some(verify) = update.verify {
        data.verify = verify
    }
    if let Some(ignore_errors) = update.ignore_errors {
        data.ignore_errors = ignore_errors
    }

    if let Some(skip_packages) = update.skip.skip_packages {
        data.skip.skip_packages = Some(skip_packages);
    }

    if let Some(skip_sections) = update.skip.skip_sections {
        data.skip.skip_sections = Some(skip_sections);
    }

    if let Some(weak_crypto) = update.weak_crypto {
        data.weak_crypto = Some(weak_crypto);
    }

    config.set_data(&id, "mirror", &data)?;
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
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// List configured media.
async fn list_media(config: Option<String>, param: Value) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: Vec<MediaConfig> = config.convert_to_typed_array("medium")?;

    let output_format = get_output_format(&param);
    let options = default_table_format_options()
        .column(ColumnConfig::new("id").header("ID"))
        .column(ColumnConfig::new("mountpoint"))
        .column(ColumnConfig::new("mirrors"))
        .column(ColumnConfig::new("verify"))
        .column(ColumnConfig::new("sync"));

    format_and_print_result_full(
        &mut serde_json::json!(config),
        &LIST_MEDIA_RETURN_TYPE,
        &output_format,
        &options,
    );

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            id: {
                schema: MEDIA_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Show full medium config entry.
async fn show_medium(config: Option<String>, id: String, param: Value) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let mut config = config.lookup_json("medium", &id)?;

    let output_format = get_output_format(&param);
    format_and_print_result_full(
        &mut config,
        &SHOW_MEDIUM_RETURN_TYPE,
        &output_format,
        &default_table_format_options(),
    );
    Ok(Value::Null)
}

#[api(
    protected: true,
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            data: {
                type: MediaConfig,
                flatten: true,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
)]
/// Create new medium config entry.
async fn add_medium(
    config: Option<String>,
    data: MediaConfig,
    _param: Value,
) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config)?;

    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;

    if section_config.sections.contains_key(&data.id) {
        param_bail!("name", "config section '{}' already exists.", data.id);
    }

    // TODO check mountpoint and mirrors exist?

    section_config.set_data(&data.id, "medium", &data)?;
    proxmox_offline_mirror::config::save_config(&config, &section_config)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            id: {
                schema: MEDIA_ID_SCHEMA,
            },
            "remove-data": {
                type: bool,
                description: "Remove ALL DATA on medium as well.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Remove medium config entry.
async fn remove_medium(
    config: Option<String>,
    id: String,
    remove_data: bool,
    _param: Value,
) -> Result<Value, Error> {
    let config_file = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut section_config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;
    match section_config.lookup::<MediaConfig>("medium", &id) {
        Ok(medium) => {
            if remove_data {
                let medium_base = Path::new(&medium.mountpoint);
                if !medium_base.exists() {
                    bail!("Medium mountpoint doesn't exist.");
                }
                remove_dir_all(medium_base)?;
            }

            section_config.sections.remove(&id);
        }
        _ => {
            param_bail!("id", "config section '{}' does not exist!", id);
        }
    }

    proxmox_offline_mirror::config::save_config(&config_file, &section_config)?;

    Ok(Value::Null)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
            id: {
                schema: MEDIA_ID_SCHEMA,
            },
            update: {
                type: MediaConfigUpdater,
                flatten: true,
            },
        },
    },
)]
/// Update medium config entry.
pub fn update_medium(
    update: MediaConfigUpdater,
    config: Option<String>,
    id: String,
) -> Result<(), Error> {
    let config_file = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;

    let mut data: MediaConfig = config.lookup("medium", &id)?;

    if let Some(mountpoint) = update.mountpoint {
        data.mountpoint = mountpoint
    }
    if let Some(mirrors) = update.mirrors {
        data.mirrors = mirrors
    }
    if let Some(sync) = update.sync {
        data.sync = sync
    }
    if let Some(verify) = update.verify {
        data.verify = verify
    }

    config.set_data(&id, "medium", &data)?;
    proxmox_offline_mirror::config::save_config(&config_file, &config)?;

    Ok(())
}

pub fn config_commands() -> CommandLineInterface {
    let mirror_cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_MIRROR))
        .insert("add", CliCommand::new(&API_METHOD_ADD_MIRROR))
        .insert("show", CliCommand::new(&API_METHOD_SHOW_MIRROR))
        .insert("remove", CliCommand::new(&API_METHOD_REMOVE_MIRROR))
        .insert("update", CliCommand::new(&API_METHOD_UPDATE_MIRROR));

    let media_cmd_def = CliCommandMap::new()
        .insert("list", CliCommand::new(&API_METHOD_LIST_MEDIA))
        .insert("add", CliCommand::new(&API_METHOD_ADD_MEDIUM))
        .insert("show", CliCommand::new(&API_METHOD_SHOW_MEDIUM))
        .insert("remove", CliCommand::new(&API_METHOD_REMOVE_MEDIUM))
        .insert("update", CliCommand::new(&API_METHOD_UPDATE_MEDIUM));

    let cmd_def = CliCommandMap::new()
        .insert("media", media_cmd_def)
        .insert("mirror", mirror_cmd_def);

    cmd_def.into()
}

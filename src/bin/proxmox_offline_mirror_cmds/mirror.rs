use anyhow::{bail, format_err, Error};

use proxmox_section_config::SectionConfigData;
use proxmox_subscription::SubscriptionStatus;
use serde_json::Value;
use std::collections::HashMap;

use proxmox_router::cli::{
    format_and_print_result, get_output_format, CliCommand, CliCommandMap, CommandLineInterface,
    OUTPUT_FORMAT,
};
use proxmox_schema::api;

use proxmox_offline_mirror::{
    config::{MirrorConfig, SubscriptionKey},
    mirror,
    types::{Snapshot, MIRROR_ID_SCHEMA},
};

use super::get_config_path;

fn get_subscription_key(
    config: &SectionConfigData,
    mirror: &MirrorConfig,
) -> Result<Option<SubscriptionKey>, Error> {
    if let Some(product) = &mirror.use_subscription {
        let subscriptions: Vec<SubscriptionKey> = config.convert_to_typed_array("subscription")?;
        let key = subscriptions
            .iter()
            .find(|key| {
                if let Ok(Some(info)) = key.info() {
                    info.status == SubscriptionStatus::Active && key.product() == *product
                } else {
                    false
                }
            })
            .ok_or_else(|| {
                format_err!(
                    "Need matching active subscription key for product {product}, but none found."
                )
            })?
            .clone();
        Ok(Some(key))
    } else {
        Ok(None)
    }
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
            "dry-run": {
                type: bool,
                optional: true,
                default: false,
                description: "Only fetch indices and print summary of missing package files, don't store anything.",
            }
        },
    },
 )]
/// Create a new repository snapshot, fetching required/missing files from original repository.
async fn create_snapshot(
    config: Option<String>,
    id: String,
    dry_run: bool,
    _param: Value,
) -> Result<(), Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MirrorConfig = section_config.lookup("mirror", &id)?;

    let subscription = get_subscription_key(&section_config, &config)?;

    proxmox_offline_mirror::mirror::create_snapshot(
        config,
        &Snapshot::now(),
        subscription,
        dry_run,
    )?;

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
            "dry-run": {
                type: bool,
                optional: true,
                default: false,
                description: "Only fetch indices and print summary of missing package files, don't store anything.",
            }
        },
    },
 )]
/// Create a new repository snapshot for each configured mirror, fetching required/missing files
/// from original repository.
async fn create_snapshots(
    config: Option<String>,
    dry_run: bool,
    _param: Value,
) -> Result<(), Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let mirrors: Vec<MirrorConfig> = section_config.convert_to_typed_array("mirror")?;

    let mut results = HashMap::new();

    for mirror in mirrors {
        let mirror_id = mirror.id.clone();
        println!("\nCREATING SNAPSHOT FOR '{mirror_id}'..");
        let subscription = match get_subscription_key(&section_config, &mirror) {
            Ok(opt_key) => opt_key,
            Err(err) => {
                eprintln!("Skipping mirror '{mirror_id}' - {err})");
                results.insert(mirror_id, Err(err));
                continue;
            }
        };
        let res = proxmox_offline_mirror::mirror::create_snapshot(
            mirror,
            &Snapshot::now(),
            subscription,
            dry_run,
        );
        if let Err(err) = &res {
            eprintln!("Failed to create snapshot for '{mirror_id}' - {err}");
        }

        results.insert(mirror_id, res);
    }

    println!("\nSUMMARY:");
    for (mirror_id, _res) in results.iter().filter(|(_, res)| res.is_ok()) {
        println!("{mirror_id}: OK"); // TODO update once we have a proper return value
    }

    let mut fail = false;

    for (mirror_id, res) in results.into_iter().filter(|(_, res)| res.is_err()) {
        fail = true;
        eprintln!("{mirror_id}: ERR - {}", res.unwrap_err());
    }

    if fail {
        bail!("Failed to create snapshots for all configured mirrors.");
    }

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
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        },
    },
 )]
/// List existing repository snapshots.
async fn list_snapshots(config: Option<String>, id: String, param: Value) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MirrorConfig = config.lookup("mirror", &id)?;

    let list = mirror::list_snapshots(&config)?;

    if output_format == "text" {
        println!("Found {} snapshots:", list.len());
        for snap in &list {
            println!("- {snap}");
        }
    } else {
        let list = serde_json::json!(list);
        format_and_print_result(&list, &output_format);
    }

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
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
            snapshot: {
                type: Snapshot,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Remove a single snapshot dir from a mirror. To actually removed the referenced data a garbage collection is required.
async fn remove_snapshot(
    config: Option<String>,
    id: String,
    snapshot: Snapshot,
    _param: Value,
) -> Result<(), Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MirrorConfig = config.lookup("mirror", &id)?;
    mirror::remove_snapshot(&config, &snapshot)?;

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
/// Run Garbage Collection on pool
async fn garbage_collect(config: Option<String>, id: String, _param: Value) -> Result<(), Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MirrorConfig = config.lookup("mirror", &id)?;

    let (count, size) = mirror::gc(&config)?;

    println!("Removed {} files totalling {}b", count, size);

    Ok(())
}
pub fn mirror_commands() -> CommandLineInterface {
    let snapshot_cmds = CliCommandMap::new()
        .insert(
            "create",
            CliCommand::new(&API_METHOD_CREATE_SNAPSHOT).arg_param(&["id"]),
        )
        .insert("create-all", CliCommand::new(&API_METHOD_CREATE_SNAPSHOTS))
        .insert(
            "list",
            CliCommand::new(&API_METHOD_LIST_SNAPSHOTS).arg_param(&["id"]),
        )
        .insert(
            "remove",
            CliCommand::new(&API_METHOD_REMOVE_SNAPSHOT).arg_param(&["id", "snapshot"]),
        );

    let cmd_def = CliCommandMap::new()
        .insert("snapshot", snapshot_cmds)
        .insert(
            "gc",
            CliCommand::new(&API_METHOD_GARBAGE_COLLECT).arg_param(&["id"]),
        );

    cmd_def.into()
}

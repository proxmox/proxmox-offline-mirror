use std::path::Path;

use anyhow::Error;
use serde_json::Value;

use proxmox_router::cli::{CliCommand, CliCommandMap, CommandLineInterface, OUTPUT_FORMAT};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_subscription::SubscriptionInfo;
use proxmox_time::epoch_to_rfc3339_utc;

use proxmox_offline_mirror::{
    config::{MediaConfig, MirrorConfig, SubscriptionKey},
    generate_repo_file_line,
    medium::{self},
    mirror,
    types::{ProductType, Snapshot, MEDIA_ID_SCHEMA},
};

use super::DEFAULT_CONFIG_PATH;

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
/// Garbage collect all mirrors on a medium.
async fn garbage_collect(
    config: Option<String>,
    id: String,
    _param: Value,
) -> Result<Value, Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MediaConfig = section_config.lookup("medium", &id)?;

    medium::gc(&config)?;

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
/// Print status of a medium
async fn status(config: Option<String>, id: String, _param: Value) -> Result<Value, Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let medium_config: MediaConfig = section_config.lookup("medium", &id)?;

    let (state, mirror_state) = medium::status(&medium_config)?;

    println!(
        "Last sync timestamp: {}",
        epoch_to_rfc3339_utc(state.last_sync)?
    );

    println!("Already synced mirrors: {:?}", mirror_state.synced);

    if !mirror_state.source_only.is_empty() {
        println!("Missing mirrors: {:?}", mirror_state.source_only);
    }

    if !mirror_state.target_only.is_empty() {
        println!("To-be-removed mirrors: {:?}", mirror_state.target_only);
    }

    for (ref id, ref mirror) in state.mirrors {
        println!("\nMirror '{}'", id);
        let mirror_config: MirrorConfig = section_config.lookup("mirror", id)?;
        let print_snapshots = |snapshots: &[Snapshot]| {
            match (snapshots.first(), snapshots.last()) {
                (Some(first), Some(last)) if first == last => {
                    println!("\t1 snapshot: {}", first);
                }
                (Some(first), Some(last)) => {
                    println!("\t{} snapshots: '{first}..{last}'", snapshots.len());
                }
                _ => {
                    println!("\tNo snapshots.");
                }
            };
        };

        let mut source_snapshots = mirror::list_snapshots(&mirror_config)?;
        source_snapshots.sort();
        println!("Source:");
        print_snapshots(&source_snapshots);
        println!("\trepository config: '{}'", mirror.repository);

        let path = Path::new(&medium_config.mountpoint);
        let mut snapshots = medium::list_snapshots(path, id)?;
        snapshots.sort();
        println!("Medium:");
        print_snapshots(&snapshots);
        if let Some(last) = snapshots.last() {
            println!(
                "\trepository config: {}",
                generate_repo_file_line(path, id, mirror, last)?
            );
        }
    }

    Ok(Value::Null)
}

fn get_subscription_keys(
    section_config: &SectionConfigData,
) -> Result<Vec<SubscriptionInfo>, Error> {
    let config_subscriptions: Vec<SubscriptionKey> =
        section_config.convert_to_typed_array("subscription")?;

    let mut subscription_infos = Vec::new();
    for subscription in config_subscriptions {
        if subscription.product() == ProductType::Pom {
            continue;
        }

        match subscription.info() {
            Ok(Some(info)) => {
                eprintln!(
                    "Including key '{}' for server '{}' with status '{}'",
                    subscription.key, subscription.server_id, info.status
                );
                subscription_infos.push(info)
            }
            Ok(None) => eprintln!(
                "No subscription info available for '{}' - run `refresh`.",
                subscription.key
            ),
            Err(err) => eprintln!(
                "Failed to parse subscription info of '{}' - {err}",
                subscription.key
            ),
        }
    }
    eprintln!(
        "will sync {} subscription keys to medium",
        subscription_infos.len()
    );
    Ok(subscription_infos)
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
            "keys-only": {
                type: bool,
                default: false,
                description: "Only sync offline subscription keys, skip repository contents",
                optional: true,
            },
        }
    },
 )]
/// Sync a medium
async fn sync(
    config: Option<String>,
    id: String,
    keys_only: bool,
    _param: Value,
) -> Result<Value, Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MediaConfig = section_config.lookup("medium", &id)?;

    let subscription_infos = get_subscription_keys(&section_config)?;

    if keys_only {
        medium::sync_keys(&config, subscription_infos)?;
    } else {
        let mut mirrors = Vec::with_capacity(config.mirrors.len());
        for mirror in &config.mirrors {
            let mirror: MirrorConfig = section_config.lookup("mirror", mirror)?;
            mirrors.push(mirror);
        }

        medium::sync(&config, mirrors, subscription_infos)?;
    }

    Ok(Value::Null)
}

pub fn medium_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert(
            "gc",
            CliCommand::new(&API_METHOD_GARBAGE_COLLECT).arg_param(&["id"]),
        )
        .insert(
            "status",
            CliCommand::new(&API_METHOD_STATUS).arg_param(&["id"]),
        )
        .insert("sync", CliCommand::new(&API_METHOD_SYNC).arg_param(&["id"]));

    cmd_def.into()
}

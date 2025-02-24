use std::path::{Path, PathBuf};

use anyhow::Error;
use serde_json::Value;

use proxmox_router::cli::{CliCommand, CliCommandMap, CommandLineInterface, OUTPUT_FORMAT};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_subscription::{ProductType, SubscriptionInfo};
use proxmox_time::epoch_to_rfc3339_utc;

use proxmox_offline_mirror::{
    config::{MediaConfig, MirrorConfig, SubscriptionKey},
    generate_repo_file_line,
    medium::{self},
    mirror,
    types::{MEDIA_ID_SCHEMA, Snapshot},
};

use super::get_config_path;

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
    let config = config.unwrap_or_else(get_config_path);

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
    let config = config.unwrap_or_else(get_config_path);

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
    let config = config.unwrap_or_else(get_config_path);

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
            verbose: {
                type: bool,
                optional: true,
                default: false,
                description: "Verbose output (print paths in addition to summary)."
            },
        }
    },
 )]
/// Diff a medium
async fn diff(
    config: Option<String>,
    id: String,
    verbose: bool,
    _param: Value,
) -> Result<Value, Error> {
    let config = config.unwrap_or_else(get_config_path);

    let (section_config, _digest) = proxmox_offline_mirror::config::config(&config)?;
    let config: MediaConfig = section_config.lookup("medium", &id)?;
    let mut mirrors = Vec::with_capacity(config.mirrors.len());
    for mirror in &config.mirrors {
        let mirror: MirrorConfig = section_config.lookup("mirror", mirror)?;
        mirrors.push(mirror);
    }

    let mut diffs = medium::diff(&config, mirrors)?;
    let mut mirrors: Vec<String> = diffs.keys().cloned().collect();
    mirrors.sort_unstable();

    let sort_paths =
        |(path, _): &(PathBuf, u64), (other_path, _): &(PathBuf, u64)| path.cmp(other_path);

    let mut first = true;
    for mirror in mirrors {
        if first {
            first = false;
        } else {
            println!();
        }

        println!("Mirror '{mirror}'");
        if let Some(Some(mut diff)) = diffs.remove(&mirror) {
            let mut total_size = 0;
            println!("\t{} file(s) only on medium:", diff.added.paths.len());
            if verbose {
                diff.added.paths.sort_unstable_by(sort_paths);
                diff.changed.paths.sort_unstable_by(sort_paths);
                diff.removed.paths.sort_unstable_by(sort_paths);
            }
            for (path, size) in diff.added.paths {
                if verbose {
                    println!("\t\t{path:?}: +{size}b");
                }
                total_size += size;
            }
            println!("\tTotal size: +{total_size}b");

            total_size = 0;
            println!(
                "\n\t{} file(s) missing on medium:",
                diff.removed.paths.len()
            );
            for (path, size) in diff.removed.paths {
                if verbose {
                    println!("\t\t{path:?}: -{size}b");
                }
                total_size += size;
            }
            println!("\tTotal size: -{total_size}b");

            total_size = 0;
            println!(
                "\n\t{} file(s) diff between source and medium:",
                diff.changed.paths.len()
            );
            for (path, size) in diff.changed.paths {
                if verbose {
                    println!("\t\t{path:?}: +-{size}b");
                }
            }
            println!("\tSum of size differences: +-{total_size}b");
        } else {
            // TODO
            println!("\tNot yet synced or no longer available on source side.");
        }
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
        .insert("sync", CliCommand::new(&API_METHOD_SYNC).arg_param(&["id"]))
        .insert("diff", CliCommand::new(&API_METHOD_DIFF).arg_param(&["id"]));

    cmd_def.into()
}

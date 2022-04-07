use std::path::Path;

use anyhow::Error;
use proxmox_time::epoch_to_rfc3339_utc;
use serde_json::Value;

use proxmox_router::cli::{CliCommand, CliCommandMap, CommandLineInterface, OUTPUT_FORMAT};
use proxmox_schema::api;

use proxmox_apt_mirror::{
    config::{MediaConfig, MirrorConfig},
    generate_repo_file_line,
    medium::{self},
    types::MIRROR_ID_SCHEMA,
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
                schema: MIRROR_ID_SCHEMA,
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

    let (section_config, _digest) = proxmox_apt_mirror::config::config(&config)?;
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
                schema: MIRROR_ID_SCHEMA,
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

    let (section_config, _digest) = proxmox_apt_mirror::config::config(&config)?;
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
        let path = Path::new(&medium_config.mountpoint);
        let snapshots = medium::list_snapshots(path, id)?;
        let repo_line = match snapshots.last() {
            None => {
                println!("no snapshots");
                None
            }
            Some(last) => {
                if let Some(first) = snapshots.first() {
                    if first == last {
                        println!("1 snapshot: '{last}'");
                    } else {
                        println!("{} snapshots: '{first}..{last}'", snapshots.len());
                    }
                    Some(generate_repo_file_line(path, id, mirror, last)?)
                } else {
                    None
                }
            }
        };
        println!("Original repository config: '{}'", mirror.repository);
        if let Some(repo_line) = repo_line {
            println!("Medium repository line: '{repo_line}'");
        }
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
                schema: MIRROR_ID_SCHEMA,
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
 )]
/// Sync a medium
async fn sync(config: Option<String>, id: String, _param: Value) -> Result<Value, Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (section_config, _digest) = proxmox_apt_mirror::config::config(&config)?;
    let config: MediaConfig = section_config.lookup("medium", &id)?;

    let mut mirrors = Vec::with_capacity(config.mirrors.len());
    for mirror in &config.mirrors {
        let mirror: MirrorConfig = section_config.lookup("mirror", mirror)?;
        mirrors.push(mirror);
    }

    medium::sync(&config, mirrors)?;

    Ok(Value::Null)
}

pub fn medium_commands() -> CommandLineInterface {
    let cmd_def = CliCommandMap::new()
        .insert("gc", CliCommand::new(&API_METHOD_GARBAGE_COLLECT))
        .insert("status", CliCommand::new(&API_METHOD_STATUS))
        .insert("sync", CliCommand::new(&API_METHOD_SYNC));

    cmd_def.into()
}

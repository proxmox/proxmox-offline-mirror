use std::path::Path;

use anyhow::Error;

use nix::libc;
use serde_json::Value;

use proxmox_router::cli::{
    default_table_format_options, format_and_print_result_full, get_output_format, CliCommand,
    CliCommandMap, CommandLineInterface, OUTPUT_FORMAT,
};
use proxmox_schema::{api, ApiStringFormat, ArraySchema, ReturnType, Schema, StringSchema};

use proxmox_apt_mirror::{
    config::MirrorConfig,
    pool::Pool,
    types::{MIRROR_ID_SCHEMA, SNAPSHOT_REGEX},
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
        },
    },
 )]
/// Create a new repository snapshot, fetching required/missing files from original repository.
async fn create_snapshot(config: Option<String>, id: String, _param: Value) -> Result<(), Error> {
    //let output_format = get_output_format(&param);
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (config, _digest) = proxmox_apt_mirror::config::config(&config)?;
    let config = config.lookup("mirror", &id)?;

    proxmox_apt_mirror::mirror::mirror(config)?;

    Ok(())
}

const SNAPSHOT_SCHEMA: Schema = StringSchema::new("Mirror snapshot")
    .format(&ApiStringFormat::Pattern(&SNAPSHOT_REGEX))
    .schema();

const LIST_SNAPSHOTS_RETURN_TYPE: ReturnType = ReturnType {
    schema: &ArraySchema::new("Returns the list of snapshots.", &SNAPSHOT_SCHEMA).schema(),
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
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
        },
    },
 )]
/// List existing repository snapshots.
async fn list_snapshots(config: Option<String>, id: String, param: Value) -> Result<(), Error> {
    let output_format = get_output_format(&param);
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (config, _digest) = proxmox_apt_mirror::config::config(&config)?;
    let config: MirrorConfig = config.lookup("mirror", &id)?;

    let _pool: Pool = (&config).try_into()?;
    let mut list = vec![];

    let path = Path::new(&config.base_dir);

    proxmox_sys::fs::scandir(
        libc::AT_FDCWD,
        path,
        &SNAPSHOT_REGEX,
        |_l2_fd, snapshot, file_type| {
            if file_type != nix::dir::Type::Directory {
                return Ok(());
            }

            list.push(snapshot.to_string());

            Ok(())
        },
    )?;
    let mut list = serde_json::json!(list);

    format_and_print_result_full(
        &mut list,
        &LIST_SNAPSHOTS_RETURN_TYPE,
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
            id: {
                schema: MIRROR_ID_SCHEMA,
            },
            snapshot: {
                schema: SNAPSHOT_SCHEMA,
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
    snapshot: String,
    _param: Value,
) -> Result<(), Error> {
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (config, _digest) = proxmox_apt_mirror::config::config(&config)?;
    let config: MirrorConfig = config.lookup("mirror", &id)?;
    let pool: Pool = (&config).try_into()?;
    let path = pool.get_path(Path::new(&snapshot))?;

    pool.lock()?.remove_dir(&path)?;

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
    let config = config.unwrap_or_else(|| DEFAULT_CONFIG_PATH.to_string());

    let (config, _digest) = proxmox_apt_mirror::config::config(&config)?;
    let config: MirrorConfig = config.lookup("mirror", &id)?;
    let pool: Pool = (&config).try_into()?;

    let (count, size) = pool.lock()?.gc()?;
    println!("Removed {} files totalling {}b", count, size);

    Ok(())
}
pub fn mirror_commands() -> CommandLineInterface {
    let snapshot_cmds = CliCommandMap::new()
        .insert("create", CliCommand::new(&API_METHOD_CREATE_SNAPSHOT))
        .insert("list", CliCommand::new(&API_METHOD_LIST_SNAPSHOTS))
        .insert("remove", CliCommand::new(&API_METHOD_REMOVE_SNAPSHOT));

    let cmd_def = CliCommandMap::new()
        .insert("snapshot", snapshot_cmds)
        .insert("gc", CliCommand::new(&API_METHOD_GARBAGE_COLLECT));

    cmd_def.into()
}

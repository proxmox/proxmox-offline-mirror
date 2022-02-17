use anyhow::Error;
use serde_json::Value;

use proxmox_apt::repositories::APTRepository;
use proxmox_router::cli::{
    run_cli_command, CliCommand, CliCommandMap, CliEnvironment, OUTPUT_FORMAT,
};
use proxmox_schema::api;
use proxmox_sys::fs::file_get_contents;

#[api(
    input: {
        properties: {
            repository: {
                type: String,
                description: "Repository string to parse.",
            },
            key: {
                type: String,
                description: "Path to repository key."
            },
            architectures: {
                type: Array,
                items: {
                    type: String,
                    description: "Architecture string (e.g., 'all', 'amd64', ..)",
                },
                description: "Architectures to mirror (default: 'all' and 'amd64')",
                optional: true,
            },
            path: {
                type: String,
                description: "Output path. Contents will be re-used if still valid.",
            },
            "output-format": {
                schema: OUTPUT_FORMAT,
                optional: true,
            },
        }
    },
     returns: {
         type: APTRepository,
     },
 )]
/// Parse a repository line.
async fn mirror(
    repository: String,
    key: String,
    architectures: Option<Vec<String>>,
    path: String,
    _param: Value,
) -> Result<Value, Error> {
    //let output_format = get_output_format(&param);

    let repository = proxmox_apt_mirror::parse_repo(repository)?;
    let key = file_get_contents(&key)?;
    let architectures = architectures.unwrap_or_else(|| vec!["amd64".to_owned(), "all".to_owned()]);

    let config = proxmox_apt_mirror::config::MirrorConfig {
        repository,
        key,
        path,
        architectures,
    };

    proxmox_apt_mirror::mirror(&config)?;

    Ok(Value::Null)
}

fn main() {
    let rpcenv = CliEnvironment::new();

    let mirror_cmd_def = CliCommand::new(&API_METHOD_MIRROR);

    let cmd_def = CliCommandMap::new().insert("mirror", mirror_cmd_def);

    run_cli_command(
        cmd_def,
        rpcenv,
        Some(|future| proxmox_async::runtime::main(future)),
    );
}

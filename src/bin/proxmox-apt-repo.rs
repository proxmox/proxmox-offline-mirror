use std::path::PathBuf;
use std::process::Command;
use std::{collections::HashMap, path::Path};

use anyhow::{bail, format_err, Error};

use proxmox_offline_mirror::types::{ProductType, Snapshot};
use proxmox_subscription::SubscriptionInfo;
use proxmox_sys::command::run_command;
use proxmox_sys::fs::{replace_file, CreateOptions};
use proxmox_sys::{fs::file_get_contents, linux::tty};
use proxmox_time::epoch_to_rfc3339_utc;
use serde_json::Value;

use proxmox_router::cli::{run_cli_command, CliCommand, CliCommandMap, CliEnvironment};
use proxmox_schema::{api, param_bail};

use proxmox_offline_mirror::helpers::tty::{
    read_bool_from_tty, read_selection_from_tty, read_string_from_tty,
};
use proxmox_offline_mirror::medium::{self, generate_repo_snippet, MediumState};

fn set_subscription_key(
    product: ProductType,
    subscription: &SubscriptionInfo,
) -> Result<String, Error> {
    let data = base64::encode(serde_json::to_vec(subscription)?);

    let cmd = match product {
        ProductType::Pve => {
            let mut cmd = Command::new("pvesubscription");
            cmd.arg("set-offline-key");
            cmd.arg(data);
            cmd
        }
        ProductType::Pbs => {
            let mut cmd = Command::new("proxmox-backup-manager");
            cmd.arg("subscription");
            cmd.arg("set-offline-key");
            cmd.arg(data);
            cmd
        }
        ProductType::Pmg => todo!(),
        ProductType::Pom => unreachable!(),
    };

    run_command(cmd, Some(|v| v == 0))
}

#[api(
    input: {
        properties: {
        },
    },
)]
/// Interactive setup wizard.
async fn setup(_param: Value) -> Result<(), Error> {
    if !tty::stdin_isatty() {
        bail!("Setup wizard can only run interactively.");
    }

    let default_dir = std::env::current_exe().map_or_else(
        |_| None,
        |mut p| {
            p.pop();
            let p = p.to_str();
            p.map(str::to_string)
        },
    );

    let mountpoint = read_string_from_tty("Path to medium mountpoint", default_dir.as_deref())?;
    let mountpoint = Path::new(&mountpoint);
    if !mountpoint.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let mut statefile = mountpoint.to_path_buf();
    statefile.push(".mirror-state");

    println!("Loading state from {statefile:?}..");
    let raw = file_get_contents(&statefile)?;
    let state: MediumState = serde_json::from_slice(&raw)?;
    println!(
        "Last sync timestamp: {}",
        epoch_to_rfc3339_utc(state.last_sync)?
    );

    let mut selected_repos = HashMap::new();

    enum Action {
        SelectMirrorSnapshot,
        DeselectMirrorSnapshot,
        GenerateSourcesList,
        UpdateOfflineSubscription,
        Quit,
    }
    let actions = &[
        (
            Action::SelectMirrorSnapshot,
            "Add mirror & snapshot to selected repositories.",
        ),
        (
            Action::DeselectMirrorSnapshot,
            "Remove mirror & snapshot from selected repositories.",
        ),
        (
            Action::GenerateSourcesList,
            "Generate 'sources.list.d' snippet for accessing selected repositories.",
        ),
        (
            Action::UpdateOfflineSubscription,
            "Update offline subscription key",
        ),
        (Action::Quit, "Quit."),
    ];

    loop {
        println!();
        if selected_repos.is_empty() {
            println!("No repositories selected so far.");
        } else {
            println!("Selected repositories:");
            for (mirror, (_info, snapshot)) in selected_repos.iter() {
                println!("\t- {mirror}/{snapshot}");
            }
        }
        println!();

        let action = read_selection_from_tty("Select action", actions, Some(0))?;
        println!();

        match action {
            Action::SelectMirrorSnapshot => {
                let mirrors: Vec<(&str, &str)> = state
                    .mirrors
                    .keys()
                    .filter_map(|k| {
                        if selected_repos.contains_key(k) {
                            None
                        } else {
                            Some((k.as_ref(), k.as_ref()))
                        }
                    })
                    .collect();

                if mirrors.is_empty() {
                    println!("All mirrors already selected.");
                    continue;
                }

                let selected_mirror = read_selection_from_tty("Select mirror", &mirrors, None)?;
                let snapshots: Vec<(Snapshot, String)> =
                    medium::list_snapshots(mountpoint, selected_mirror)?
                        .into_iter()
                        .map(|s| (s, s.to_string()))
                        .collect();
                if snapshots.is_empty() {
                    println!("Mirror doesn't have any synced snapshots.");
                    continue;
                }

                let snapshots: Vec<(&Snapshot, &str)> = snapshots
                    .iter()
                    .map(|(snap, string)| (snap, string.as_ref()))
                    .collect();
                let selected_snapshot = read_selection_from_tty(
                    "Select snapshot",
                    &snapshots,
                    Some(snapshots.len() - 1),
                )?;

                selected_repos.insert(
                    selected_mirror.to_string(),
                    (
                        state.mirrors.get(*selected_mirror).unwrap(),
                        **selected_snapshot,
                    ),
                );
            }
            Action::DeselectMirrorSnapshot => {
                let mirrors: Vec<(&str, &str)> = selected_repos
                    .keys()
                    .map(|k| (k.as_ref(), k.as_ref()))
                    .collect();

                let selected_mirror =
                    read_selection_from_tty("Deselect mirror", &mirrors, None)?.to_string();
                selected_repos.remove(&selected_mirror);
            }
            Action::GenerateSourcesList => {
                let lines = generate_repo_snippet(mountpoint, &selected_repos)?;
                println!("Generated sources.list.d snippet:");
                let data = lines.join("\n");
                println!();
                println!("-----8<-----");
                println!("{data}");
                println!("----->8-----");
                if read_bool_from_tty("Configure snippet as repository source", Some(true))? {
                    let snippet_file_name = loop {
                        let file = read_string_from_tty(
                            "Enter filename under '/etc/apt/sources.list.d/' (will be overwritten)",
                            Some("offline-mirror.list"),
                        )?;
                        if file.contains("/") {
                            eprintln!("Invalid file name.");
                        } else {
                            break file;
                        }
                    };
                    let mut file = PathBuf::from("/etc/apt/sources.list.d");
                    file.push(snippet_file_name);
                    replace_file(file, data.as_bytes(), CreateOptions::default(), true)?;
                } else {
                    println!("Add above snippet to system's repository entries (/etc/apt/sources.list.d/) manually to configure.");
                }

                println!("Now run 'apt update && apt full-upgrade' to upgrade system.");
                println!();
            }
            Action::UpdateOfflineSubscription => {
                let server_id = proxmox_subscription::get_hardware_address()?;
                let subscriptions: Vec<(&SubscriptionInfo, &str)> = state
                    .subscriptions
                    .iter()
                    .filter_map(|s| {
                        if let Some(key) = s.key.as_ref() {
                            if let Ok(product) = key[..3].parse::<ProductType>() {
                                if product == ProductType::Pom {
                                    return None;
                                } else {
                                    return Some((s, key.as_str()));
                                }
                            }
                        }
                        return None;
                    })
                    .collect();

                if subscriptions.is_empty() {
                    println!(
                        "No matching subscription key found for server ID '{}'",
                        server_id
                    );
                } else {
                    let info = read_selection_from_tty("Select key", &subscriptions, None)?;
                    // safe unwrap, checked above!
                    let product: ProductType = info.key.as_ref().unwrap()[..3].parse()?;
                    set_subscription_key(product, info)?;
                }
            }
            Action::Quit => {
                break;
            }
        }
    }

    Ok(())
}

#[api(
    input: {
        properties: {
            mountpoint: {
                type: String,
                optional: true,
                description: "Path to medium mountpoint - defaults to `proxmox-apt-repo` containing directory.",
            },
            product: {
                type: ProductType,
            },
        },
    },
)]
/// Configures and offline subscription key
async fn setup_offline_key(
    mountpoint: Option<String>,
    product: ProductType,
    _param: Value,
) -> Result<(), Error> {
    if product == ProductType::Pom {
        param_bail!(
            "product",
            format_err!("Proxmox Offline Mirror does not support offline operations.")
        );
    }

    let mountpoint = mountpoint
        .or_else(|| {
            std::env::current_exe().map_or_else(
                |_| None,
                |mut p| {
                    p.pop();
                    let p = p.to_str();
                    p.map(str::to_string)
                },
            )
        })
        .ok_or_else(|| {
            format_err!("Failed to determine fallback mountpoint via executable path.")
        })?;

    let mountpoint = Path::new(&mountpoint);
    if !mountpoint.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let mut statefile = mountpoint.to_path_buf();
    statefile.push(".mirror-state");

    println!("Loading state from {statefile:?}..");
    let raw = file_get_contents(&statefile)?;
    let state: MediumState = serde_json::from_slice(&raw)?;
    println!(
        "Last sync timestamp: {}",
        epoch_to_rfc3339_utc(state.last_sync)?
    );

    let server_id = proxmox_subscription::get_hardware_address()?;
    let subscription = state.subscriptions.iter().find(|s| {
        if let Some(key) = s.key.as_ref() {
            if let Ok(found_product) = key[..3].parse::<ProductType>() {
                return product == found_product;
            }
        }
        return false;
    });

    match subscription {
        Some(subscription) => {
            eprintln!("Setting offline subscription key for {product}..");
            match set_subscription_key(product, subscription) {
                Ok(output) if !output.is_empty() => eprintln!("success: {output}"),
                Ok(_) => eprintln!("success."),
                Err(err) => eprintln!("error: {err}"),
            }
            Ok(())
        }
        None => bail!("No matching subscription key found for product '{product}' and server ID '{server_id}'"),
    }
}

fn main() {
    let rpcenv = CliEnvironment::new();

    let cmd_def = CliCommandMap::new()
        .insert("setup", CliCommand::new(&API_METHOD_SETUP))
        .insert(
            "offline-key",
            CliCommand::new(&API_METHOD_SETUP_OFFLINE_KEY),
        );

    run_cli_command(
        cmd_def,
        rpcenv,
        Some(|future| proxmox_async::runtime::main(future)),
    );
}

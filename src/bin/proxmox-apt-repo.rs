use std::{collections::HashMap, path::Path};

use anyhow::{bail, Error};

use proxmox_apt_mirror::types::Snapshot;
use proxmox_sys::{fs::file_get_contents, linux::tty};
use proxmox_time::epoch_to_rfc3339_utc;
use serde_json::Value;

use proxmox_router::cli::{run_cli_command, CliCommand, CliCommandMap, CliEnvironment};
use proxmox_schema::api;

use proxmox_apt_mirror::helpers::tty::{read_selection_from_tty, read_string_from_tty};
use proxmox_apt_mirror::medium::{self, generate_repo_snippet, MediumState};

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
        PrintSourcesList,
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
            Action::PrintSourcesList,
            "Print 'sources.list.d' snippet for accessing selected repositories.",
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
            Action::PrintSourcesList => {
                let lines = generate_repo_snippet(mountpoint, &selected_repos)?;
                println!(
                    "Put the following into '/etc/apt/sources.list.d/proxmox-apt-mirror.list'"
                );
                println!();
                println!("-----8<-----");
                println!("{}", lines.join("\n"));
                println!("----->8-----");
                println!("And run 'apt update && apt full-upgrade'");
                println!();
            }
            Action::Quit => {
                break;
            }
        }
    }

    Ok(())
}
fn main() {
    let rpcenv = CliEnvironment::new();

    let cmd_def = CliCommandMap::new().insert("setup", CliCommand::new(&API_METHOD_SETUP));

    run_cli_command(
        cmd_def,
        rpcenv,
        Some(|future| proxmox_async::runtime::main(future)),
    );
}

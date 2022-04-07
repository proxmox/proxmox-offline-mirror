use std::path::Path;

use anyhow::{bail, Error};
use serde_json::Value;

use proxmox_router::cli::{run_cli_command, CliCommand, CliCommandMap, CliEnvironment};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_sys::linux::tty;

use proxmox_apt_mirror::helpers::tty::{
    read_bool_from_tty, read_selection_from_tty, read_string_from_tty,
};
use proxmox_apt_mirror::{
    config::{save_config, MediaConfig, MirrorConfig},
    mirror,
    types::MIRROR_ID_SCHEMA,
};

mod proxmox_apt_mirror_cmds;
use proxmox_apt_mirror_cmds::*;

fn action_add_mirror(config: &SectionConfigData) -> Result<MirrorConfig, Error> {
    let (repository, key_path, architectures) = if read_bool_from_tty("Guided Setup", Some(true))? {
        enum Distro {
            Debian,
            Pbs,
            Pmg,
            Pve,
            PveCeph,
        }

        let distros = &[
            (Distro::Debian, "Debian"),
            (Distro::Pbs, "Proxmox Backup Server"),
            (Distro::Pmg, "Proxmox Mail Gateway"),
            (Distro::Pve, "Proxmox VE"),
            (Distro::PveCeph, "Proxmox VE Ceph"),
        ];
        let dist = read_selection_from_tty("Select distro to mirror", distros, None)?;

        enum Release {
            Bullseye,
            Buster,
        }

        let releases = &[(Release::Bullseye, "Bullseye"), (Release::Buster, "Buster")];
        let release = read_selection_from_tty("Select release", releases, Some(0))?;

        let (url, key_path) = match dist {
            Distro::Debian => {
                enum DebianVariant {
                    Main,
                    Security,
                    Updates,
                    Backports,
                    Debug,
                }

                let variants = &[
                    (DebianVariant::Main, "Main repository"),
                    (DebianVariant::Security, "Security"),
                    (DebianVariant::Updates, "Updates"),
                    (DebianVariant::Backports, "Backports"),
                    (DebianVariant::Debug, "Debug Information"),
                ];
                let variant =
                    read_selection_from_tty("Select repository variant", variants, Some(0))?;
                let components = read_string_from_tty(
                    "Enter repository components",
                    Some("main contrib non-free"),
                )?;

                let url = match (release, variant) {
                    (Release::Bullseye, DebianVariant::Main) => {
                        "http://deb.debian.org/debian bullseye"
                    }
                    (Release::Bullseye, DebianVariant::Security) => {
                        "http://deb.debian.org/debian-security bullseye-security"
                    }
                    (Release::Bullseye, DebianVariant::Updates) => {
                        "http://deb.debian.org/debian bullseye-updates"
                    }
                    (Release::Bullseye, DebianVariant::Backports) => {
                        "http://deb.debian.org/debian bullseye-backports"
                    }
                    (Release::Bullseye, DebianVariant::Debug) => {
                        "http://deb.debian.org/debian-debug bullseye-debug"
                    }
                    (Release::Buster, DebianVariant::Main) => "http://deb.debian.org/debian buster",
                    (Release::Buster, DebianVariant::Security) => {
                        "http://deb.debian.org/debian-security buster/updates"
                    }
                    (Release::Buster, DebianVariant::Updates) => {
                        "http://deb.debian.org/debian buster-updates"
                    }
                    (Release::Buster, DebianVariant::Backports) => {
                        "http://deb.debian.org/debian buster-backports"
                    }
                    (Release::Buster, DebianVariant::Debug) => {
                        "http://deb.debian.org/debian-debug buster-debug"
                    }
                };

                let url = format!("{url} {components}");
                let key = match (release, variant) {
                    (Release::Bullseye, DebianVariant::Security) => {
                        "/usr/share/keyrings/debian-archive-bullseye-security-automatic.gpg"
                    }
                    (Release::Bullseye, _) => {
                        "/usr/share/keyrings/debian-archive-bullseye-stable.gpg"
                    }
                    (Release::Buster, DebianVariant::Security) => {
                        "/usr/share/keyrings/debian-archive-buster-security-automatic.gpg"
                    }
                    (Release::Buster, _) => "/usr/share/keyrings/debian-archive-buster-stable.gpg",
                };

                (url, key.to_string())
            }
            Distro::PveCeph => {
                enum CephRelease {
                    Luminous,
                    Nautilus,
                    Octopus,
                    Pacific,
                }

                let releases = match release {
                    Release::Bullseye => {
                        vec![
                            (CephRelease::Octopus, "Octopus (15.x)"),
                            (CephRelease::Pacific, "Pacific (16.x)"),
                        ]
                    }
                    Release::Buster => {
                        vec![
                            (CephRelease::Luminous, "Luminous (12.x)"),
                            (CephRelease::Nautilus, "Nautilus (14.x)"),
                            (CephRelease::Octopus, "Octopus (15.x)"),
                        ]
                    }
                };

                let ceph_release = read_selection_from_tty(
                    "Select Ceph release",
                    &releases,
                    Some(releases.len() - 1),
                )?;

                let components =
                    read_string_from_tty("Enter repository components", Some("main test"))?;

                let key = match release {
                    Release::Bullseye => "/etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg",
                    Release::Buster => "/etc/apt/trusted.gpg.d/proxmox-release-buster.gpg",
                };

                let release = match release {
                    Release::Bullseye => "bullseye",
                    Release::Buster => "buster",
                };

                let ceph_release = match ceph_release {
                    CephRelease::Luminous => "luminous",
                    CephRelease::Nautilus => "nautilus",
                    CephRelease::Octopus => "octopus",
                    CephRelease::Pacific => "pacific",
                };

                let url = format!(
                    "http://download.proxmox.com/debian/ceph-{ceph_release} {release} {components}"
                );

                (url, key.to_string())
            }
            proxmox_product => {
                enum ProxmoxVariant {
                    Enterprise,
                    NoSubscription,
                    Test,
                }

                let variants = &[
                    (ProxmoxVariant::Enterprise, "Enterprise repository"),
                    (ProxmoxVariant::NoSubscription, "No-Subscription repository"),
                    (ProxmoxVariant::Test, "Test repository"),
                ];

                let variant =
                    read_selection_from_tty("Select repository variant", variants, Some(0))?;

                let product = match proxmox_product {
                    Distro::Pbs => "pbs",
                    Distro::Pmg => "pmg",
                    Distro::Pve => "pve",
                    _ => {
                        bail!("Invalid");
                    }
                };

                // TODO enterprise query for key!
                let url = match (release, variant) {
                    (Release::Bullseye, ProxmoxVariant::Enterprise) => format!("https://enterprise.proxmox.com/debian/{product} bullseye {product}-enterprise"),
                    (Release::Bullseye, ProxmoxVariant::NoSubscription) => format!("http://download.proxmox.com/debian/{product} bullseye {product}-no-subscription"),
                    (Release::Bullseye, ProxmoxVariant::Test) => format!("http://download.proxmox.com/debian/{product} bullseye {product}test"),
                    (Release::Buster, ProxmoxVariant::Enterprise) => format!("https://enterprise.proxmox.com/debian/{product} buster {product}-enterprise"),
                    (Release::Buster, ProxmoxVariant::NoSubscription) => format!("http://download.proxmox.com/debian/{product} buster {product}-no-subscription"),
                    (Release::Buster, ProxmoxVariant::Test) => format!("http://download.proxmox.com/debian/{product} buster {product}test"),
                };

                let key = match release {
                    Release::Bullseye => "/etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg",
                    Release::Buster => "/etc/apt/trusted.gpg.d/proxmox-release-buster.gpg",
                };

                (url, key.to_string())
            }
        };

        let architectures = vec!["amd64".to_string(), "all".to_string()];
        (format!("deb {url}"), key_path, architectures)
    } else {
        let repo = read_string_from_tty("Enter repository line in sources.list format", None)?;
        let key_path = read_string_from_tty("Enter path to repository key file", None)?;
        let architectures =
            read_string_from_tty("Enter list of architectures to mirror", Some("amd64,all"))?;
        let architectures: Vec<String> = architectures
            .split(|c: char| c == ',' || c.is_ascii_whitespace())
            .filter_map(|value| {
                if value.is_empty() {
                    None
                } else {
                    Some(value.to_owned())
                }
            })
            .collect();
        (repo, key_path, architectures)
    };

    if !Path::new(&key_path).exists() {
        eprintln!("Keyfile '{key_path}' doesn't exist - make sure to install relevant keyring packages or update config to provide correct path!");
    }

    let id = loop {
        let mut id = read_string_from_tty("Enter mirror ID", None)?;
        while let Err(err) = MIRROR_ID_SCHEMA.parse_simple_value(&id) {
            eprintln!("Not a valid mirror ID: {err}");
            id = read_string_from_tty("Enter mirror ID", None)?;
        }

        if config.sections.contains_key(&id) {
            eprintln!("Config entry '{id}' already exists!");
            continue;
        }

        break id;
    };

    let dir = loop {
        let path =
            read_string_from_tty("Enter path where mirrored repository will be stored", None)?;
        if Path::new(&path).exists() {
            eprintln!("Path already exists.");
        } else {
            break path;
        }
    };

    let verify = read_bool_from_tty(
        "Should already mirrored files be re-verified when updating the mirror? (io-intensive!)",
        Some(true),
    )?;
    let sync = read_bool_from_tty("Should newly written files be written using FSYNC to ensure crash-consistency? (io-intensive!)", Some(true))?;

    Ok(MirrorConfig {
        id,
        repository,
        architectures,
        key_path,
        verify,
        sync,
        dir,
    })
}

fn action_add_medium(config: &SectionConfigData) -> Result<MediaConfig, Error> {
    let mountpoint = loop {
        let path = read_string_from_tty("Enter path where medium is mounted", None)?;
        let mountpoint = Path::new(&path);
        if !mountpoint.exists() {
            eprintln!("Path doesn't exist.");
        } else {
            let mut statefile = mountpoint.to_path_buf();
            statefile.push(".mirror-state");
            if !statefile.exists()
                || read_bool_from_tty(
                    &format!("Found existing statefile at {statefile:?} - proceed?"),
                    Some(false),
                )?
            {
                break path;
            }
        }
    };

    let mirrors: Vec<MirrorConfig> = config.convert_to_typed_array("mirror")?;
    let mut available_mirrors: Vec<String> = Vec::new();
    for mirror_config in mirrors {
        available_mirrors.push(mirror_config.id);
    }

    let mut selected_mirrors: Vec<String> = Vec::new();

    enum Action {
        SelectMirror,
        DeselectMirror,
        Proceed,
    }
    let actions = &[
        (Action::SelectMirror, "Add mirror to selected mirrors."),
        (
            Action::DeselectMirror,
            "Remove mirror from selected mirrors.",
        ),
        (Action::Proceed, "Proceed"),
    ];

    loop {
        println!();
        if selected_mirrors.is_empty() {
            println!("No mirrors selected so far.");
        } else {
            println!("Selected mirrors:");
            for id in &selected_mirrors {
                println!("\t- {id}");
            }
        }
        println!();

        let action = read_selection_from_tty("Select action", actions, Some(0))?;
        println!();

        match action {
            Action::SelectMirror => {
                if available_mirrors.is_empty() {
                    println!("No unselected mirrors available.");
                    continue;
                }

                let mirrors: Vec<(&str, &str)> = available_mirrors
                    .iter()
                    .map(|v| (v.as_ref(), v.as_ref()))
                    .collect();

                let selected =
                    read_selection_from_tty("Select a mirror to add", &mirrors, None)?.to_string();
                available_mirrors = available_mirrors
                    .into_iter()
                    .filter(|v| *v != selected)
                    .collect();
                selected_mirrors.push(selected);
            }
            Action::DeselectMirror => {
                if selected_mirrors.is_empty() {
                    println!("No selected mirrors available.");
                    continue;
                }

                let mirrors: Vec<(&str, &str)> = selected_mirrors
                    .iter()
                    .map(|v| (v.as_ref(), v.as_ref()))
                    .collect();

                let selected =
                    read_selection_from_tty("Select a mirror to remove", &mirrors, None)?
                        .to_string();
                selected_mirrors = selected_mirrors
                    .into_iter()
                    .filter(|v| *v != selected)
                    .collect();
                available_mirrors.push(selected);
            }
            Action::Proceed => {
                break;
            }
        }
    }

    let verify = read_bool_from_tty(
        "Should mirrored files be re-verified when updating the medium? (io-intensive!)",
        Some(true),
    )?;
    let sync = read_bool_from_tty("Should newly written files be written using FSYNC to ensure crash-consistency? (io-intensive!)", Some(true))?;

    let id = loop {
        let mut id = read_string_from_tty("Enter medium ID", None)?;
        while let Err(err) = MIRROR_ID_SCHEMA.parse_simple_value(&id) {
            eprintln!("Not a valid medium ID: {err}");
            id = read_string_from_tty("Enter medium ID", None)?;
        }

        if config.sections.contains_key(&id) {
            eprintln!("Config entry '{id}' already exists!");
            continue;
        }

        break id;
    };

    Ok(MediaConfig {
        id,
        mountpoint,
        mirrors: selected_mirrors,
        verify,
        sync,
    })
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

    let config_file = read_string_from_tty("Mirror config file", Some(DEFAULT_CONFIG_PATH))?;
    let _lock = proxmox_apt_mirror::config::lock_config(&config_file)?;

    let (mut config, _digest) = proxmox_apt_mirror::config::config(&config_file)?;

    if config.sections.is_empty() {
        println!("Initializing new config.");
    } else {
        println!("Loaded existing config.");
    }

    enum Action {
        AddMirror,
        AddMedium,
        Quit,
    }

    let actions = &[
        (Action::AddMirror, "Add new mirror entry"),
        (Action::AddMedium, "Add new medium entry"),
        (Action::Quit, "Quit"),
    ];

    loop {
        println!();
        if !config.sections.is_empty() {
            println!("Existing config entries:");
            for (section, (section_type, _)) in config.sections.iter() {
                println!("{section_type} '{section}'");
            }
            println!();
        }

        match read_selection_from_tty("Select Action:", actions, Some(0))? {
            Action::Quit => break,
            Action::AddMirror => {
                let mirror_config = action_add_mirror(&config)?;
                let id = mirror_config.id.clone();
                mirror::init(&mirror_config)?;
                config.set_data(&id, "mirror", mirror_config)?;
                save_config(&config_file, &config)?;
                println!("Config entry '{id}' added");
                println!("Run \"proxmox-apt-mirror mirror snapshot create --config '{config_file}' --id '{id}'\" to create a new mirror snapshot.");
            }
            Action::AddMedium => {
                let media_config = action_add_medium(&config)?;
                let id = media_config.id.clone();
                config.set_data(&id, "medium", media_config)?;
                save_config(&config_file, &config)?;
                println!("Config entry '{id}' added");
                println!("Run \"proxmox-apt-mirror medium sync --config '{config_file}' --id '{id}'\" to sync mirror snapshots to medium.");
            }
        }
    }

    Ok(())
}
fn main() {
    let rpcenv = CliEnvironment::new();

    let cmd_def = CliCommandMap::new()
        .insert("setup", CliCommand::new(&API_METHOD_SETUP))
        .insert("config", config_commands())
        .insert("medium", medium_commands())
        .insert("mirror", mirror_commands());

    run_cli_command(
        cmd_def,
        rpcenv,
        Some(|future| proxmox_async::runtime::main(future)),
    );
}

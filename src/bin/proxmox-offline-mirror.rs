use std::fmt::Display;
use std::matches;
use std::path::Path;

use anyhow::{bail, format_err, Error};
use proxmox_offline_mirror::config::SubscriptionKey;
use proxmox_offline_mirror::subscription::{extract_mirror_key, refresh_mirror_key};
use serde_json::Value;

use proxmox_router::cli::{run_cli_command, CliCommand, CliCommandMap, CliEnvironment};
use proxmox_schema::api;
use proxmox_section_config::SectionConfigData;
use proxmox_sys::linux::tty;

use proxmox_offline_mirror::helpers::tty::{
    read_bool_from_tty, read_selection_from_tty, read_string_from_tty,
};
use proxmox_offline_mirror::{
    config::{save_config, MediaConfig, MirrorConfig, SkipConfig},
    mirror,
    types::{ProductType, MEDIA_ID_SCHEMA, MIRROR_ID_SCHEMA},
};

mod proxmox_offline_mirror_cmds;
use proxmox_offline_mirror_cmds::*;

enum Distro {
    Debian,
    Pbs,
    Pmg,
    Pve,
    PveCeph,
}

impl Display for Distro {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Distro::Debian => write!(f, "debian"),
            Distro::Pbs => write!(f, "pbs"),
            Distro::Pmg => write!(f, "pmg"),
            Distro::Pve => write!(f, "pve"),
            Distro::PveCeph => write!(f, "ceph"),
        }
    }
}

enum Release {
    Bookworm,
    Bullseye,
    Buster,
}

impl Display for Release {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Release::Bookworm => write!(f, "bookworm"),
            Release::Bullseye => write!(f, "bullseye"),
            Release::Buster => write!(f, "buster"),
        }
    }
}

enum DebianVariant {
    Main,
    Security,
    Updates,
    Backports,
    Debug,
}

impl Display for DebianVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DebianVariant::Main => write!(f, "main"),
            DebianVariant::Security => write!(f, "security"),
            DebianVariant::Updates => write!(f, "updates"),
            DebianVariant::Backports => write!(f, "backports"),
            DebianVariant::Debug => write!(f, "debug"),
        }
    }
}

#[derive(PartialEq)]
enum ProxmoxVariant {
    Enterprise,
    NoSubscription,
    Test,
}

impl Display for ProxmoxVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxmoxVariant::Enterprise => write!(f, "enterprise"),
            ProxmoxVariant::NoSubscription => write!(f, "no_subscription"),
            ProxmoxVariant::Test => write!(f, "test"),
        }
    }
}

fn derive_debian_repo(
    release: &Release,
    variant: &DebianVariant,
    components: &str,
) -> Result<(String, String, String, SkipConfig), Error> {
    println!("Configure filters for Debian mirror {release} / {variant}:");
    let skip_sections = match read_string_from_tty(
        "\tEnter list of package sections to be skipped ('-' for None)",
        Some("debug,games"),
    )?
    .as_str()
    {
        "-" => None,
        list => Some(
            list.split(',')
                .map(|v| v.trim().to_owned())
                .collect::<Vec<String>>(),
        ),
    };
    let skip_packages = match read_string_from_tty(
        "\tEnter list of package names/name globs to be skipped ('-' for None)",
        None,
    )?
    .as_str()
    {
        "-" => None,
        list => Some(
            list.split(',')
                .map(|v| v.trim().to_owned())
                .collect::<Vec<String>>(),
        ),
    };
    let filters = SkipConfig {
        skip_packages,
        skip_sections,
    };
    let url = match (release, variant) {
        (Release::Bookworm, DebianVariant::Main) => "http://deb.debian.org/debian bookworm",
        (Release::Bookworm, DebianVariant::Security) => {
            "http://deb.debian.org/debian-security bookworm-security"
        }
        (Release::Bookworm, DebianVariant::Updates) => {
            "http://deb.debian.org/debian bookworm-updates"
        }
        (Release::Bookworm, DebianVariant::Backports) => {
            "http://deb.debian.org/debian bookworm-backports"
        }
        (Release::Bookworm, DebianVariant::Debug) => {
            "http://deb.debian.org/debian-debug bookworm-debug"
        }
        (Release::Bullseye, DebianVariant::Main) => "http://deb.debian.org/debian bullseye",
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
        (Release::Buster, DebianVariant::Updates) => "http://deb.debian.org/debian buster-updates",
        (Release::Buster, DebianVariant::Backports) => {
            "http://deb.debian.org/debian buster-backports"
        }
        (Release::Buster, DebianVariant::Debug) => {
            "http://deb.debian.org/debian-debug buster-debug"
        }
    };

    let url = format!("{url} {components}");
    let key = match (release, variant) {
        (Release::Bookworm, DebianVariant::Security) => {
            "/usr/share/keyrings/debian-archive-bookworm-security-automatic.gpg"
        }
        (Release::Bookworm, DebianVariant::Updates) |
        (Release::Bookworm, DebianVariant::Backports) => {
            "/usr/share/keyrings/debian-archive-bookworm-automatic.gpg"
        }
        (Release::Bookworm, _) => "/usr/share/keyrings/debian-archive-bookworm-stable.gpg",
        (Release::Bullseye, DebianVariant::Security) => {
            "/usr/share/keyrings/debian-archive-bullseye-security-automatic.gpg"
        }
        (Release::Bullseye, _) => "/usr/share/keyrings/debian-archive-bullseye-automatic.gpg",
        (Release::Buster, DebianVariant::Security) => {
            "/usr/share/keyrings/debian-archive-buster-security-automatic.gpg"
        }
        (Release::Buster, _) => "/usr/share/keyrings/debian-archive-buster-stable.gpg",
    };

    let suggested_id = format!("debian_{release}_{variant}");

    Ok((url, key.to_string(), suggested_id, filters))
}

fn action_add_mirror(config: &SectionConfigData) -> Result<Vec<MirrorConfig>, Error> {
    let mut use_subscription = None;
    let mut extra_repos = Vec::new();

    let (repository, key_path, architectures, suggested_id, skip) = if read_bool_from_tty(
        "Guided Setup",
        Some(true),
    )? {
        let distros = &[
            (Distro::Pve, "Proxmox VE"),
            (Distro::Pbs, "Proxmox Backup Server"),
            (Distro::Pmg, "Proxmox Mail Gateway"),
            (Distro::PveCeph, "Proxmox Ceph"),
            (Distro::Debian, "Debian"),
        ];
        let dist = read_selection_from_tty("Select distro to mirror", distros, None)?;

        let releases = &[
            (Release::Bookworm, "Bookworm"),
            (Release::Bullseye, "Bullseye"),
            (Release::Buster, "Buster"),
        ];
        let release = read_selection_from_tty("Select release", releases, Some(0))?;

        let mut add_debian_repo = false;

        let (url, key_path, suggested_id, skip) = match dist {
            Distro::Debian => {
                let variants = &[
                    (DebianVariant::Main, "Main repository"),
                    (DebianVariant::Security, "Security"),
                    (DebianVariant::Updates, "Updates"),
                    (DebianVariant::Backports, "Backports"),
                    (DebianVariant::Debug, "Debug Information"),
                ];
                let variant =
                    read_selection_from_tty("Select repository variant", variants, Some(0))?;

                let default_components = match release {
                    Release::Bookworm => "main contrib non-free non-free-firmware",
                    _ => "main contrib non-free"
                };

                let components = read_string_from_tty(
                    "Enter repository components",
                    Some(default_components),
                )?;

                derive_debian_repo(release, variant, &components)?
            }
            Distro::PveCeph => {
                enum CephRelease {
                    Luminous,
                    Nautilus,
                    Octopus,
                    Pacific,
                    Quincy,
                    Reef,
                }

                let releases = match release {
                    Release::Bookworm => vec![
                        (CephRelease::Quincy, "Quincy (17.x)"),
                        (CephRelease::Reef, "Reef (18.x)"),
                    ],
                    Release::Bullseye => {
                        vec![
                            (CephRelease::Octopus, "Octopus (15.x)"),
                            (CephRelease::Pacific, "Pacific (16.x)"),
                            (CephRelease::Quincy, "Quincy (17.x)"),
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

                let (base_url, components) = if matches!(release, Release::Bookworm) {
                    let variants = &[
                        (ProxmoxVariant::Enterprise, "Enterprise repository"),
                        (ProxmoxVariant::NoSubscription, "No-Subscription repository"),
                        (ProxmoxVariant::Test, "Test repository"),
                    ];

                    let variant =
                        read_selection_from_tty("Select repository variant", variants, Some(0))?;

                    match variant {
                        ProxmoxVariant::Enterprise => {
                            use_subscription = Some(ProductType::Pve);
                            (
                                "https://enterprise.proxmox.com/debian/ceph",
                                "enterprise".to_string(),
                            )
                        }
                        ProxmoxVariant::NoSubscription => (
                            "http://download.proxmox.com/debian/ceph",
                            "no-subscription".to_string(),
                        ),
                        ProxmoxVariant::Test => (
                            "http://download.proxmox.com/debian/ceph",
                            "test".to_string(),
                        ),
                    }
                } else {
                    (
                        "http://download.proxmox.com/debian/ceph",
                        read_string_from_tty("Enter repository components", Some("main test"))?,
                    )
                };

                let key = match release {
                    Release::Bookworm => "/etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg",
                    Release::Bullseye => "/etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg",
                    Release::Buster => "/etc/apt/trusted.gpg.d/proxmox-release-buster.gpg",
                };

                let ceph_release = match ceph_release {
                    CephRelease::Luminous => "luminous",
                    CephRelease::Nautilus => "nautilus",
                    CephRelease::Octopus => "octopus",
                    CephRelease::Pacific => "pacific",
                    CephRelease::Quincy => "quincy",
                    CephRelease::Reef => "reef",
                };

                let url = format!("{base_url}-{ceph_release} {release} {components}");
                let suggested_id = format!("ceph_{ceph_release}_{release}");

                (url, key.to_string(), suggested_id, SkipConfig::default())
            }
            product => {
                let variants = &[
                    (ProxmoxVariant::Enterprise, "Enterprise repository"),
                    (ProxmoxVariant::NoSubscription, "No-Subscription repository"),
                    (ProxmoxVariant::Test, "Test repository"),
                ];

                let variant =
                    read_selection_from_tty("Select repository variant", variants, Some(0))?;

                // TODO enterprise query for key!
                let url = match (release, variant) {
                    (Release::Bookworm, ProxmoxVariant::Enterprise) => format!("https://enterprise.proxmox.com/debian/{product} bookworm {product}-enterprise"),
                    (Release::Bookworm, ProxmoxVariant::NoSubscription) => format!("http://download.proxmox.com/debian/{product} bookworm {product}-no-subscription"),
                    (Release::Bookworm, ProxmoxVariant::Test) => format!("http://download.proxmox.com/debian/{product} bookworm {product}test"),
                    (Release::Bullseye, ProxmoxVariant::Enterprise) => format!("https://enterprise.proxmox.com/debian/{product} bullseye {product}-enterprise"),
                    (Release::Bullseye, ProxmoxVariant::NoSubscription) => format!("http://download.proxmox.com/debian/{product} bullseye {product}-no-subscription"),
                    (Release::Bullseye, ProxmoxVariant::Test) => format!("http://download.proxmox.com/debian/{product} bullseye {product}test"),
                    (Release::Buster, ProxmoxVariant::Enterprise) => format!("https://enterprise.proxmox.com/debian/{product} buster {product}-enterprise"),
                    (Release::Buster, ProxmoxVariant::NoSubscription) => format!("http://download.proxmox.com/debian/{product} buster {product}-no-subscription"),
                    (Release::Buster, ProxmoxVariant::Test) => format!("http://download.proxmox.com/debian/{product} buster {product}test"),
                };

                use_subscription = match (product, variant) {
                    (Distro::Pbs, &ProxmoxVariant::Enterprise) => Some(ProductType::Pbs),
                    (Distro::Pmg, &ProxmoxVariant::Enterprise) => Some(ProductType::Pmg),
                    (Distro::Pve, &ProxmoxVariant::Enterprise) => Some(ProductType::Pve),
                    _ => None,
                };

                let key = match release {
                    Release::Bookworm => "/etc/apt/trusted.gpg.d/proxmox-release-bookworm.gpg",
                    Release::Bullseye => "/etc/apt/trusted.gpg.d/proxmox-release-bullseye.gpg",
                    Release::Buster => "/etc/apt/trusted.gpg.d/proxmox-release-buster.gpg",
                };

                let suggested_id = format!("{product}_{release}_{variant}");

                add_debian_repo = read_bool_from_tty(
                    "Should missing Debian mirrors for the selected product be auto-added",
                    Some(true),
                )?;

                (url, key.to_string(), suggested_id, SkipConfig::default())
            }
        };

        let architectures = vec!["amd64".to_string(), "all".to_string()];

        if add_debian_repo {
            extra_repos.push(derive_debian_repo(
                release,
                &DebianVariant::Main,
                "main contrib",
            )?);
            extra_repos.push(derive_debian_repo(
                release,
                &DebianVariant::Updates,
                "main contrib",
            )?);
            extra_repos.push(derive_debian_repo(
                release,
                &DebianVariant::Security,
                "main contrib",
            )?);
        }
        (
            format!("deb {url}"),
            key_path,
            architectures,
            Some(suggested_id),
            skip,
        )
    } else {
        let repo = read_string_from_tty("Enter repository line in sources.list format", None)?;
        let key_path = read_string_from_tty("Enter (absolute) path to repository key file", None)?;
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
        let subscription_products = &[
            (Some(ProductType::Pve), "PVE"),
            (Some(ProductType::Pbs), "PBS"),
            (Some(ProductType::Pmg), "PMG"),
            (None, "None"),
        ];
        use_subscription = read_selection_from_tty(
            "Does this repository require a valid Proxmox subscription key",
            subscription_products,
            None,
        )?
        .clone();

        (repo, key_path, architectures, None, SkipConfig::default())
    };

    if !Path::new(&key_path).exists() {
        eprintln!("Keyfile '{key_path}' doesn't exist - make sure to install relevant keyring packages or update config to provide correct path!");
    }

    let id = loop {
        let mut id = read_string_from_tty("Enter mirror ID", suggested_id.as_deref())?;
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

    let base_dir = loop {
        let path = read_string_from_tty(
            "Enter (absolute) base path where mirrored repositories will be stored",
            Some("/var/lib/proxmox-offline-mirror/mirrors/"),
        )?;
        if !path.starts_with('/') {
            eprintln!("Path must start with '/'");
        } else {
            break path;
        }
    };

    let verify = read_bool_from_tty(
        "Should already mirrored files be re-verified when updating the mirror? (io-intensive!)",
        Some(true),
    )?;
    let sync = read_bool_from_tty("Should newly written files be written using FSYNC to ensure crash-consistency? (io-intensive!)", Some(true))?;

    let mut configs = Vec::with_capacity(extra_repos.len() + 1);

    for (url, key_path, suggested_id, skip) in extra_repos {
        if config.sections.contains_key(&suggested_id) {
            eprintln!("config section '{suggested_id}' already exists, skipping..");
        } else {
            let repository = format!("deb {url}");

            configs.push(MirrorConfig {
                id: suggested_id,
                repository,
                architectures: architectures.clone(),
                key_path,
                verify,
                sync,
                base_dir: base_dir.clone(),
                use_subscription: None,
                ignore_errors: false,
                skip,
                weak_crypto: None,
            });
        }
    }

    let main_config = MirrorConfig {
        id,
        repository,
        architectures,
        key_path,
        verify,
        sync,
        base_dir,
        use_subscription,
        ignore_errors: false,
        skip,
        weak_crypto: None,
    };

    configs.push(main_config);
    Ok(configs)
}

fn action_add_medium(config: &SectionConfigData) -> Result<MediaConfig, Error> {
    let id = loop {
        let id = read_string_from_tty("Enter new medium ID", None)?;
        if let Err(err) = MEDIA_ID_SCHEMA.parse_simple_value(&id) {
            eprintln!("Not a valid medium ID: {err}");
            continue;
        }

        if config.sections.contains_key(&id) {
            eprintln!("Config entry '{id}' already exists!");
            continue;
        }

        break id;
    };

    let mountpoint = loop {
        let path = read_string_from_tty("Enter (absolute) path where medium is mounted", None)?;
        if !path.starts_with('/') {
            eprintln!("Path must start with '/'");
            continue;
        }

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
        SelectAllMirrors,
        DeselectMirror,
        DeselectAllMirrors,
        Proceed,
    }

    loop {
        println!();
        let actions = if selected_mirrors.is_empty() {
            println!("No mirrors selected for inclusion on medium so far.");
            vec![
                (Action::SelectMirror, "Add mirror to selection."),
                (Action::SelectAllMirrors, "Add all mirrors to selection."),
                (Action::Proceed, "Proceed"),
            ]
        } else {
            println!("Mirrors selected for inclusion on medium:");
            for id in &selected_mirrors {
                println!("\t- {id}");
            }
            println!();
            if available_mirrors.is_empty() {
                println!("No more mirrors available for selection!");
                vec![
                    (Action::DeselectMirror, "Remove mirror from selection."),
                    (
                        Action::DeselectAllMirrors,
                        "Remove all mirrors from selection.",
                    ),
                    (Action::Proceed, "Proceed"),
                ]
            } else {
                vec![
                    (Action::SelectMirror, "Add mirror to selection."),
                    (Action::SelectAllMirrors, "Add all mirrors to selection."),
                    (Action::DeselectMirror, "Remove mirror from selection."),
                    (
                        Action::DeselectAllMirrors,
                        "Remove all mirrors from selection.",
                    ),
                    (Action::Proceed, "Proceed"),
                ]
            }
        };

        println!();

        let action = read_selection_from_tty("Select action", &actions, Some(0))?;
        println!();

        match action {
            Action::SelectMirror => {
                if available_mirrors.is_empty() {
                    println!("No (more) unselected mirrors available.");
                    continue;
                }

                let mirrors: Vec<(&str, &str)> = available_mirrors
                    .iter()
                    .map(|v| (v.as_ref(), v.as_ref()))
                    .collect();

                let selected =
                    read_selection_from_tty("Select a mirror to add", &mirrors, None)?.to_string();
                available_mirrors.retain(|v| *v != selected);
                selected_mirrors.push(selected);
            }
            Action::SelectAllMirrors => {
                selected_mirrors.extend_from_slice(&available_mirrors);
                available_mirrors.truncate(0);
            }
            Action::DeselectMirror => {
                if selected_mirrors.is_empty() {
                    println!("No mirrors selected (yet).");
                    continue;
                }

                let mirrors: Vec<(&str, &str)> = selected_mirrors
                    .iter()
                    .map(|v| (v.as_ref(), v.as_ref()))
                    .collect();

                let selected =
                    read_selection_from_tty("Select a mirror to remove", &mirrors, None)?
                        .to_string();
                selected_mirrors.retain(|v| *v != selected);
                available_mirrors.push(selected);
            }
            Action::DeselectAllMirrors => {
                available_mirrors.extend_from_slice(&selected_mirrors);
                selected_mirrors.truncate(0);
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

    Ok(MediaConfig {
        id,
        mountpoint,
        mirrors: selected_mirrors,
        verify,
        sync,
    })
}

fn action_add_key(config: &SectionConfigData) -> Result<SubscriptionKey, Error> {
    let (product, mirror_key) = if let Ok(mirror_key) =
        extract_mirror_key(&config.convert_to_typed_array("subscription")?)
    {
        let subscription_products = &[
            (ProductType::Pve, "Proxmox VE"),
            (ProductType::Pbs, "Proxmox Backup Server"),
            (ProductType::Pmg, "Proxmox Mail Gateway"),
        ];

        let product = read_selection_from_tty(
            "Select Proxmox product for which subscription key should be added",
            subscription_products,
            None,
        )?;

        (product, Some(mirror_key))
    } else {
        println!("No mirror key configured yet, forcing mirror key setup first..");
        (&ProductType::Pom, None)
    };

    let key = read_string_from_tty("Please enter subscription key", None)?;
    if config.sections.get(&key).is_some() {
        bail!("Key entry for '{key}' already exists - please use 'key refresh' or 'key update'!");
    }

    let server_id = if product == &ProductType::Pom {
        let server_id = proxmox_subscription::get_hardware_address()?;
        println!("Server ID of this system is '{server_id}'");
        server_id
    } else {
        read_string_from_tty(
            "Please enter server ID of offline system using this subscription",
            None,
        )?
    };

    let mut data = SubscriptionKey {
        key,
        server_id,
        description: None,
        info: None,
    };

    if data.product() != *product {
        bail!(
            "Selected product and product in subscription key don't match: {} != {}",
            product,
            data.product()
        );
    }

    if read_bool_from_tty("Attempt to refresh key", Some(true))? {
        let info = if let Some(mirror_key) = mirror_key {
            if let Err(err) = refresh_mirror_key(mirror_key.clone()) {
                eprintln!("Failed to refresh mirror_key '{}' - {err}", mirror_key.key);
            }

            let mut refreshed = proxmox_offline_mirror::subscription::refresh_offline_keys(
                mirror_key,
                vec![data.clone()],
                public_key()?,
            )?;

            refreshed
                .pop()
                .ok_or_else(|| format_err!("Server did not return subscription info.."))?
        } else {
            proxmox_offline_mirror::subscription::refresh_mirror_key(data.clone())?
        };

        println!(
            "Refreshed subscription info - status: {}, message: {}",
            info.status,
            info.message.as_ref().unwrap_or(&"-".to_string())
        );

        if info.key.as_ref() == Some(&data.key) {
            data.info = Some(base64::encode(serde_json::to_vec(&info)?));
        } else {
            bail!("Server returned subscription info for wrong key.");
        }
    }

    Ok(data)
}

#[api(
    input: {
        properties: {
            config: {
                type: String,
                optional: true,
                description: "Path to mirroring config file.",
            },
        },
    },
)]
/// Interactive setup wizard.
async fn setup(config: Option<String>, _param: Value) -> Result<(), Error> {
    if !tty::stdin_isatty() {
        bail!("Setup wizard can only run interactively.");
    }

    let config_file = config.unwrap_or_else(get_config_path);

    let _lock = proxmox_offline_mirror::config::lock_config(&config_file)?;

    let (mut config, _digest) = proxmox_offline_mirror::config::config(&config_file)?;

    if config.sections.is_empty() {
        println!("Initializing new config.");
    } else {
        println!("Loaded existing config.");
    }

    enum Action {
        AddKey,
        AddMirror,
        AddMedium,
        Quit,
    }

    loop {
        println!();
        let mut mirror_defined = false;
        if !config.sections.is_empty() {
            println!("Existing config entries:");
            for (section, (section_type, _)) in config.sections.iter() {
                if section_type == "mirror" {
                    mirror_defined = true;
                }
                println!("{section_type} '{section}'");
            }
            println!();
        }

        let actions = if mirror_defined {
            vec![
                (Action::AddMirror, "Add new mirror entry"),
                (Action::AddMedium, "Add new medium entry"),
                (Action::AddKey, "Add new subscription key"),
                (Action::Quit, "Quit"),
            ]
        } else {
            vec![
                (Action::AddMirror, "Add new mirror entry"),
                (Action::AddKey, "Add new subscription key"),
                (Action::Quit, "Quit"),
            ]
        };

        match read_selection_from_tty("Select Action:", &actions, Some(0))? {
            Action::Quit => break,
            Action::AddMirror => {
                for mirror_config in action_add_mirror(&config)? {
                    let id = mirror_config.id.clone();
                    mirror::init(&mirror_config)?;
                    config.set_data(&id, "mirror", mirror_config)?;
                    save_config(&config_file, &config)?;
                    println!("Config entry '{id}' added");
                    println!("Run \"proxmox-offline-mirror mirror snapshot create --config '{config_file}' '{id}'\" to create a new mirror snapshot.");
                }
            }
            Action::AddMedium => {
                let media_config = action_add_medium(&config)?;
                let id = media_config.id.clone();
                config.set_data(&id, "medium", media_config)?;
                save_config(&config_file, &config)?;
                println!("Config entry '{id}' added");
                println!("Run \"proxmox-offline-mirror medium sync --config '{config_file}' '{id}'\" to sync mirror snapshots to medium.");
            }
            Action::AddKey => {
                let key = action_add_key(&config)?;
                let id = key.key.clone();
                config.set_data(&id, "subscription", &key)?;
                save_config(&config_file, &config)?;
                println!("Config entry '{id}' added");
                println!("Run \"proxmox-offline-mirror key refresh\" to refresh subscription information");
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
        .insert("key", key_commands())
        .insert("medium", medium_commands())
        .insert("mirror", mirror_commands());

    run_cli_command(
        cmd_def,
        rpcenv,
        Some(|future| proxmox_async::runtime::main(future)),
    );
}

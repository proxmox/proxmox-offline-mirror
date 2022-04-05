use std::{
    collections::{HashMap, HashSet},
    path::Path,
};

use anyhow::{bail, format_err, Error};
use nix::libc;
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox_time::{epoch_i64, epoch_to_rfc3339_utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{self, MirrorConfig},
    convert_repo_line,
    pool::Pool,
    types::SNAPSHOT_REGEX,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MirrorInfo {
    repository: String,
    architectures: Vec<String>,
}

impl From<&MirrorConfig> for MirrorInfo {
    fn from(config: &MirrorConfig) -> Self {
        Self {
            repository: config.repository.clone(),
            architectures: config.architectures.clone(),
        }
    }
}

impl From<MirrorConfig> for MirrorInfo {
    fn from(config: MirrorConfig) -> Self {
        Self {
            repository: config.repository,
            architectures: config.architectures,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MediumState {
    pub mirrors: HashMap<String, MirrorInfo>,
    pub last_sync: i64,
}

pub fn list_snapshots(medium_base: &Path, mirror: &str) -> Result<Vec<String>, Error> {
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let mut list = vec![];
    let mut mirror_base = medium_base.to_path_buf();
    mirror_base.push(Path::new(&mirror));

    proxmox_sys::fs::scandir(
        libc::AT_FDCWD,
        &mirror_base,
        &SNAPSHOT_REGEX,
        |_l2_fd, snapshot, file_type| {
            if file_type != nix::dir::Type::Directory {
                return Ok(());
            }

            list.push(snapshot.to_string());

            Ok(())
        },
    )?;

    list.sort();

    Ok(list)
}

pub fn generate_repo_snippet(
    medium_base: &Path,
    repositories: &HashMap<String, (&MirrorInfo, String)>,
) -> Result<Vec<String>, Error> {
    let mut res = Vec::new();
    for (mirror_id, (mirror_info, snapshot)) in repositories {
        res.push(generate_repo_file_line(
            medium_base,
            mirror_id,
            mirror_info,
            snapshot,
        )?);
    }
    Ok(res)
}

fn generate_repo_file_line(
    medium_base: &Path,
    mirror_id: &str,
    mirror: &MirrorInfo,
    snapshot: &str,
) -> Result<String, Error> {
    let mut snapshot_path = medium_base.to_path_buf();
    snapshot_path.push(mirror_id);
    snapshot_path.push(snapshot);
    let snapshot_path = snapshot_path
        .to_str()
        .ok_or_else(|| format_err!("Failed to convert snapshot path to String"))?;

    let mut repo = convert_repo_line(mirror.repository.clone())?;
    repo.uris = vec![format!("file://{}", snapshot_path)];

    repo.options
        .push(proxmox_apt::repositories::APTRepositoryOption {
            key: "check-valid-until".to_string(),
            values: vec!["false".to_string()],
        });

    let mut res = Vec::new();
    repo.write(&mut res)?;

    let res = String::from_utf8(res)
        .map_err(|err| format_err!("Couldn't convert repo line to String - {err}"))?;

    Ok(res.trim_end().to_string())
}

pub fn gc(medium: &crate::config::MediaConfig) -> Result<(), Error> {
    let medium_base = Path::new(&medium.mountpoint);
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let mut statefile = medium_base.to_path_buf();
    statefile.push(".mirror-state");

    let _lock = config::lock_config(&format!("{}/{}", medium.mountpoint, "mirror-state"))?;

    println!("Loading state from {statefile:?}..");
    let raw = file_get_contents(&statefile)?;
    let state: MediumState = serde_json::from_slice(&raw)?;
    println!(
        "Last sync timestamp: {}",
        epoch_to_rfc3339_utc(state.last_sync)?
    );

    let mut total_count = 0usize;
    let mut total_bytes = 0_u64;

    for (id, _info) in state.mirrors {
        println!("\nGC for '{id}'");
        let mut mirror_base = medium_base.to_path_buf();
        mirror_base.push(Path::new(&id));

        let mut mirror_pool = mirror_base.clone();
        mirror_pool.push(".pool"); // TODO make configurable somehow?

        if mirror_base.exists() {
            let pool = Pool::open(&mirror_base, &mirror_pool)?;
            let locked = pool.lock()?;
            let (count, bytes) = locked.gc()?;
            println!("removed {count} files ({bytes}b)");
            total_count += count;
            total_bytes += bytes;
        } else {
            println!("{mirror_base:?} doesn't exist, skipping '{}'", id);
        };
    }

    println!("GC removed {total_count} files ({total_bytes}b)");

    Ok(())
}

pub fn status(medium: &crate::config::MediaConfig) -> Result<(), Error> {
    let medium_base = Path::new(&medium.mountpoint);
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let mut statefile = medium_base.to_path_buf();
    statefile.push(".mirror-state");

    println!("Loading state from {statefile:?}..");
    let raw = file_get_contents(&statefile)?;
    let state: MediumState = serde_json::from_slice(&raw)?;
    println!(
        "Last sync timestamp: {}",
        epoch_to_rfc3339_utc(state.last_sync)?
    );

    let synced_mirrors: HashSet<String> = state
        .mirrors
        .iter()
        .map(|(id, _mirror)| id.clone())
        .collect();
    let config_mirrors: HashSet<String> = medium.mirrors.iter().cloned().collect();
    let new_mirrors: HashSet<String> = config_mirrors
        .difference(&synced_mirrors)
        .cloned()
        .collect();
    let dropped_mirrors: HashSet<String> = synced_mirrors
        .difference(&config_mirrors)
        .cloned()
        .collect();

    println!("Already synced mirrors: {synced_mirrors:?}");
    println!("Configured mirrors: {config_mirrors:?}");

    if !new_mirrors.is_empty() {
        println!("Missing mirrors: {new_mirrors:?}");
    }

    if !dropped_mirrors.is_empty() {
        println!("To-be-removed mirrors: {dropped_mirrors:?}");
    }

    for (ref id, ref mirror) in state.mirrors {
        println!("\nMirror '{}'", id);
        let snapshots = list_snapshots(Path::new(&medium.mountpoint), id)?;
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
                    Some(generate_repo_file_line(medium_base, id, mirror, last)?)
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

    Ok(())
}

pub fn sync(medium: &crate::config::MediaConfig, mirrors: Vec<MirrorConfig>) -> Result<(), Error> {
    println!(
        "Syncing {} mirrors {:?} to medium '{}' ({:?})",
        &medium.mirrors.len(),
        &medium.mirrors,
        &medium.id,
        &medium.mountpoint
    );

    let medium_base = Path::new(&medium.mountpoint);
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let mut statefile = medium_base.to_path_buf();
    statefile.push(".mirror-state");

    let _lock = config::lock_config(&format!("{}/{}", medium.mountpoint, "mirror-state"))?;

    let mut state = if statefile.exists() {
        println!("Loading state from {statefile:?}..");
        let raw = file_get_contents(&statefile)?;
        let state: MediumState = serde_json::from_slice(&raw)?;
        println!(
            "Last sync timestamp: {}",
            epoch_to_rfc3339_utc(state.last_sync)?
        );
        state
    } else {
        println!("Creating new statefile {statefile:?}..");
        MediumState {
            mirrors: HashMap::new(),
            last_sync: 0,
        }
    };

    state.last_sync = epoch_i64();
    println!("Sync timestamp: {}", epoch_to_rfc3339_utc(state.last_sync)?);

    let old_mirrors: HashSet<String> = state
        .mirrors
        .iter()
        .map(|(id, _mirror)| id.clone())
        .collect();
    let sync_mirrors: HashSet<String> = mirrors.iter().map(|mirror| mirror.id.clone()).collect();
    let new_mirrors: HashSet<String> = sync_mirrors.difference(&old_mirrors).cloned().collect();
    let dropped_mirrors: HashSet<String> = old_mirrors.difference(&sync_mirrors).cloned().collect();

    println!("Previously synced mirrors: {:?}", &old_mirrors);

    if !new_mirrors.is_empty() {
        println!(
            "Adding {} new mirror(s) to target medium: {new_mirrors:?}",
            new_mirrors.len()
        );
    }
    if !dropped_mirrors.is_empty() {
        println!(
            "Dropping {} removed mirror(s) from target medium (after syncing): {dropped_mirrors:?}",
            dropped_mirrors.len()
        );
    }

    println!("\nStarting sync now!");
    state.mirrors = HashMap::new();

    for mirror in mirrors.into_iter() {
        let mut mirror_base = medium_base.to_path_buf();
        mirror_base.push(Path::new(&mirror.id));

        println!("\nSyncing '{}' to {mirror_base:?}..", mirror.id);

        let mut mirror_pool = mirror_base.clone();
        mirror_pool.push(".pool"); // TODO make configurable somehow?

        let target_pool = if mirror_base.exists() {
            Pool::open(&mirror_base, &mirror_pool)?
        } else {
            Pool::create(&mirror_base, &mirror_pool)?
        };

        let source_pool: Pool = (&mirror).try_into()?;
        source_pool.lock()?.sync_pool(&target_pool, medium.verify)?;

        state.mirrors.insert(mirror.id.clone(), mirror.into());
    }

    if !dropped_mirrors.is_empty() {
        println!();
    }
    for dropped in dropped_mirrors {
        let mut mirror_base = medium_base.to_path_buf();
        mirror_base.push(Path::new(&dropped));

        if mirror_base.exists() {
            println!("Removing previously synced, but no longer configured mirror '{dropped}'..");
            std::fs::remove_dir_all(&mirror_base)?;
        }
    }

    println!("Updating statefile..");
    // TODO update state file for exporting/subscription key handling/..?
    replace_file(
        &statefile,
        &serde_json::to_vec(&state)?,
        CreateOptions::default(),
        true,
    )?;

    Ok(())
}

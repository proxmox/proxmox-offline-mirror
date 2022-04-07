use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

use anyhow::{bail, format_err, Error};
use nix::libc;
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox_time::{epoch_i64, epoch_to_rfc3339_utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{self, ConfigLockGuard, MediaConfig, MirrorConfig},
    generate_repo_file_line,
    mirror::pool,
    pool::Pool,
    types::{Snapshot, SNAPSHOT_REGEX},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct MirrorInfo {
    pub repository: String,
    pub architectures: Vec<String>,
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

pub struct MediumMirrorState {
    pub synced: HashSet<String>,
    pub source_only: HashSet<String>,
    pub target_only: HashSet<String>,
}

fn get_mirror_state(config: &MediaConfig, state: &MediumState) -> MediumMirrorState {
    let synced_mirrors: HashSet<String> = state
        .mirrors
        .iter()
        .map(|(id, _mirror)| id.clone())
        .collect();
    let config_mirrors: HashSet<String> = config.mirrors.iter().cloned().collect();
    let new_mirrors: HashSet<String> = config_mirrors
        .difference(&synced_mirrors)
        .cloned()
        .collect();
    let dropped_mirrors: HashSet<String> = synced_mirrors
        .difference(&config_mirrors)
        .cloned()
        .collect();

    MediumMirrorState {
        synced: synced_mirrors,
        source_only: new_mirrors,
        target_only: dropped_mirrors,
    }
}

fn lock(base: &Path) -> Result<ConfigLockGuard, Error> {
    let mut lockfile = base.to_path_buf();
    lockfile.push("mirror-state");
    let lockfile = lockfile
        .to_str()
        .ok_or_else(|| format_err!("Couldn't convert lockfile path {lockfile:?})"))?;
    config::lock_config(lockfile)
}

fn statefile(base: &Path) -> PathBuf {
    let mut statefile = base.to_path_buf();
    statefile.push(".mirror-state");
    statefile
}

fn load_state(base: &Path) -> Result<Option<MediumState>, Error> {
    let statefile = statefile(base);

    if statefile.exists() {
        let raw = file_get_contents(&statefile)?;
        let state: MediumState = serde_json::from_slice(&raw)?;
        Ok(Some(state))
    } else {
        Ok(None)
    }
}

fn write_state(_lock: &ConfigLockGuard, base: &Path, state: &MediumState) -> Result<(), Error> {
    replace_file(
        &statefile(base),
        &serde_json::to_vec(&state)?,
        CreateOptions::default(),
        true,
    )?;

    Ok(())
}

pub fn list_snapshots(medium_base: &Path, mirror: &str) -> Result<Vec<Snapshot>, Error> {
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

            list.push(snapshot.parse()?);

            Ok(())
        },
    )?;

    list.sort();

    Ok(list)
}

pub fn generate_repo_snippet(
    medium_base: &Path,
    repositories: &HashMap<String, (&MirrorInfo, Snapshot)>,
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

pub fn gc(medium: &crate::config::MediaConfig) -> Result<(), Error> {
    let medium_base = Path::new(&medium.mountpoint);
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let _lock = lock(medium_base)?;

    println!("Loading state..");
    let state = load_state(medium_base)?
        .ok_or_else(|| format_err!("Cannot GC empty medium - no statefile found."))?;

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

pub fn status(
    medium: &crate::config::MediaConfig,
) -> Result<(MediumState, MediumMirrorState), Error> {
    let medium_base = Path::new(&medium.mountpoint);
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let state = load_state(medium_base)?
        .ok_or_else(|| format_err!("No status available - statefile doesn't exist."))?;
    let mirror_state = get_mirror_state(medium, &state);

    Ok((state, mirror_state))
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

    let lock = lock(medium_base)?;

    let mut state = match load_state(medium_base)? {
        Some(state) => {
            println!("Loaded existing statefile.");
            println!(
                "Last sync timestamp: {}",
                epoch_to_rfc3339_utc(state.last_sync)?
            );
            state
        }
        None => {
            println!("Creating new statefile..");
            MediumState {
                mirrors: HashMap::new(),
                last_sync: 0,
            }
        }
    };

    state.last_sync = epoch_i64();
    println!("Sync timestamp: {}", epoch_to_rfc3339_utc(state.last_sync)?);

    let mirror_state = get_mirror_state(medium, &state);
    println!("Previously synced mirrors: {:?}", &mirror_state.synced);

    if !mirror_state.source_only.is_empty() {
        println!(
            "Adding {} new mirror(s) to target medium: {:?}",
            mirror_state.source_only.len(),
            mirror_state.source_only,
        );
    }
    if !mirror_state.target_only.is_empty() {
        println!(
            "Dropping {} removed mirror(s) from target medium (after syncing): {:?}",
            mirror_state.target_only.len(),
            mirror_state.target_only,
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

        let source_pool: Pool = pool(&mirror)?;
        source_pool.lock()?.sync_pool(&target_pool, medium.verify)?;

        state.mirrors.insert(mirror.id.clone(), mirror.into());
    }

    if !mirror_state.target_only.is_empty() {
        println!();
    }
    for dropped in mirror_state.target_only {
        let mut mirror_base = medium_base.to_path_buf();
        mirror_base.push(Path::new(&dropped));

        if mirror_base.exists() {
            println!("Removing previously synced, but no longer configured mirror '{dropped}'..");
            std::fs::remove_dir_all(&mirror_base)?;
        }
    }

    println!("Updating statefile..");
    // TODO update state file for exporting/subscription key handling/..?
    write_state(&lock, medium_base, &state)?;

    Ok(())
}

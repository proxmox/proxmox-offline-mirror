use std::{
    collections::{HashMap, HashSet},
    fs::Metadata,
    os::linux::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::{bail, format_err, Error};
use nix::libc;
use openssl::sha::sha256;
use proxmox_subscription::SubscriptionInfo;
use proxmox_sys::fs::{file_get_contents, replace_file, CreateOptions};
use proxmox_time::{epoch_i64, epoch_to_rfc3339_utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{self, ConfigLockGuard, MediaConfig, MirrorConfig},
    generate_repo_file_line,
    mirror::pool,
    pool::Pool,
    types::{Diff, Snapshot, SNAPSHOT_REGEX},
};
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// Information about a mirror on the medium.
///
/// Used to generate repository lines for accessing the synced mirror.
pub struct MirrorInfo {
    /// Original repository line
    pub repository: String,
    /// Mirrored architectures
    pub architectures: Vec<String>,
    /// Pool directory (relative to medium base)
    pub pool: String,
}

impl From<&MirrorConfig> for MirrorInfo {
    fn from(config: &MirrorConfig) -> Self {
        Self {
            repository: config.repository.clone(),
            architectures: config.architectures.clone(),
            pool: mirror_pool_dir(config),
        }
    }
}

impl From<MirrorConfig> for MirrorInfo {
    fn from(config: MirrorConfig) -> Self {
        Self {
            pool: mirror_pool_dir(&config),
            repository: config.repository,
            architectures: config.architectures,
        }
    }
}

fn mirror_pool_dir(mirror: &MirrorConfig) -> String {
    let pool_suffix = hex::encode(sha256(mirror.base_dir.as_bytes()));
    format!(".pool_{pool_suffix}")
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
/// State of mirrors on the medium
pub struct MediumState {
    /// Map of mirror ID to `MirrorInfo`.
    pub mirrors: HashMap<String, MirrorInfo>,
    /// Timestamp of last sync operation.
    pub last_sync: i64,
    /// Subscriptions
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub subscriptions: Vec<SubscriptionInfo>,
}

/// Information about the mirrors on a medium.
///
/// Derived from `MediaConfig` (supposed state) and `MediumState` (actual state)
pub struct MediumMirrorState {
    /// Mirrors which are configured and synced
    pub synced: HashSet<String>,
    /// Mirrors which are configured
    pub config: HashSet<String>,
    /// Mirrors which are configured but not synced yet
    pub source_only: HashSet<String>,
    /// Mirrors which are not configured but exist on medium
    pub target_only: HashSet<String>,
}

// helper to derive `MediumMirrorState`
fn get_mirror_state(config: &MediaConfig, state: &MediumState) -> MediumMirrorState {
    let synced_mirrors: HashSet<String> = state.mirrors.keys().cloned().collect();
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
        config: config_mirrors,
        source_only: new_mirrors,
        target_only: dropped_mirrors,
    }
}

// Helper to lock medium
fn lock(base: &Path) -> Result<ConfigLockGuard, Error> {
    let mut lockfile = base.to_path_buf();
    lockfile.push("mirror-state");
    let lockfile = lockfile
        .to_str()
        .ok_or_else(|| format_err!("Couldn't convert lockfile path {lockfile:?})"))?;
    config::lock_config(lockfile)
}

// Helper to get statefile path
fn statefile(base: &Path) -> PathBuf {
    let mut statefile = base.to_path_buf();
    statefile.push(".mirror-state");
    statefile
}

// Helper to load statefile
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

// Helper to write statefile
fn write_state(_lock: &ConfigLockGuard, base: &Path, state: &MediumState) -> Result<(), Error> {
    replace_file(
        &statefile(base),
        &serde_json::to_vec(&state)?,
        CreateOptions::default(),
        true,
    )?;

    Ok(())
}

/// List snapshots of a given mirror on a given medium.
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

/// Generate a repository snippet for a selection of mirrors on a medium.
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

/// Run garbage collection on all mirrors on a medium.
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

    for (id, info) in state.mirrors {
        println!("\nGC for '{id}'");
        let mut mirror_base = medium_base.to_path_buf();
        mirror_base.push(Path::new(&id));

        let mut mirror_pool = medium_base.to_path_buf();
        mirror_pool.push(info.pool);

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

/// Get `MediumState` and `MediumMirrorState` for a given medium.
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

/// Sync only subscription keys to medium
pub fn sync_keys(
    medium: &crate::config::MediaConfig,
    subscriptions: Vec<SubscriptionInfo>,
) -> Result<(), Error> {
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
                subscriptions: vec![],
            }
        }
    };

    state.last_sync = epoch_i64();
    println!("Sync timestamp: {}", epoch_to_rfc3339_utc(state.last_sync)?);

    println!("Updating statefile..");
    state.subscriptions = subscriptions;
    write_state(&lock, medium_base, &state)?;

    Ok(())
}

/// Sync medium's content according to config.
pub fn sync(
    medium: &crate::config::MediaConfig,
    mirrors: Vec<MirrorConfig>,
    subscriptions: Vec<SubscriptionInfo>,
) -> Result<(), Error> {
    println!(
        "Syncing {} mirrors {:?} to medium '{}' ({:?})",
        &medium.mirrors.len(),
        &medium.mirrors,
        &medium.id,
        &medium.mountpoint
    );

    if mirrors.len() != medium.mirrors.len() {
        bail!("Number of mirrors in config and sync request don't match.");
    }

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
                subscriptions: vec![],
            }
        }
    };

    state.last_sync = epoch_i64();
    println!("Sync timestamp: {}", epoch_to_rfc3339_utc(state.last_sync)?);

    let mirror_state = get_mirror_state(medium, &state);
    println!("Previously synced mirrors: {:?}", &mirror_state.synced);

    let pools: HashMap<String, String> =
        state
            .mirrors
            .iter()
            .fold(HashMap::new(), |mut map, (id, info)| {
                map.insert(id.clone(), info.pool.clone());
                map
            });

    let requested: HashSet<String> = mirrors.iter().map(|mirror| mirror.id.clone()).collect();
    if requested != mirror_state.config {
        bail!(
            "Config and sync request don't use the same mirror list: {:?} / {:?}",
            mirror_state.config,
            requested
        );
    }

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

        let mut mirror_pool = medium_base.to_path_buf();
        let pool_dir = match pools.get(&mirror.id) {
            Some(pool_dir) => pool_dir.to_owned(),
            None => mirror_pool_dir(&mirror),
        };
        mirror_pool.push(pool_dir);

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
            match pools.get(&dropped) {
                Some(pool) => {
                    println!("Removing previously synced, but no longer configured mirror '{dropped}'..");
                    let mut pool_dir = medium_base.to_path_buf();
                    pool_dir.push(pool);
                    let pool = Pool::open(&mirror_base, &pool_dir)?;
                    pool.lock()?.destroy()?;
                },
                None => bail!("No pool information for previously synced, but no longer configured mirror '{dropped}'"),
            }
        }
    }

    println!("Updating statefile..");
    state.subscriptions = subscriptions;
    write_state(&lock, medium_base, &state)?;

    Ok(())
}

/// Sync medium's content according to config.
pub fn diff(
    medium: &crate::config::MediaConfig,
    mirrors: Vec<MirrorConfig>,
) -> Result<HashMap<String, Option<Diff>>, Error> {
    let medium_base = Path::new(&medium.mountpoint);
    if !medium_base.exists() {
        bail!("Medium mountpoint doesn't exist.");
    }

    let _lock = lock(medium_base)?;

    let state =
        load_state(medium_base)?.ok_or_else(|| format_err!("Medium not yet initializes."))?;

    let mirror_state = get_mirror_state(medium, &state);

    let pools: HashMap<String, String> =
        state
            .mirrors
            .iter()
            .fold(HashMap::new(), |mut map, (id, info)| {
                map.insert(id.clone(), info.pool.clone());
                map
            });

    let mut diffs = HashMap::new();

    let convert_file_list_to_diff = |files: Vec<(PathBuf, Metadata)>, added: bool| -> Diff {
        files
            .into_iter()
            .fold(Diff::default(), |mut diff, (file, meta)| {
                if !meta.is_file() {
                    return diff;
                }

                let size = meta.st_size();
                if added {
                    diff.added.paths.push((file, size));
                } else {
                    diff.removed.paths.push((file, size));
                }
                diff
            })
    };

    let get_target_pool =
        |mirror_id: &str, mirror: Option<&MirrorConfig>| -> Result<Option<Pool>, Error> {
            let mut mirror_base = medium_base.to_path_buf();
            mirror_base.push(Path::new(mirror_id));

            let mut mirror_pool = medium_base.to_path_buf();
            let pool_dir = match pools.get(mirror_id) {
                Some(pool_dir) => pool_dir.to_owned(),
                None => {
                    if let Some(mirror) = mirror {
                        mirror_pool_dir(mirror)
                    } else {
                        return Ok(None);
                    }
                }
            };
            mirror_pool.push(pool_dir);

            Ok(Some(Pool::open(&mirror_base, &mirror_pool)?))
        };

    for mirror in mirrors.into_iter() {
        let source_pool: Pool = pool(&mirror)?;

        if !mirror_state.synced.contains(&mirror.id) {
            let files = source_pool.lock()?.list_files()?;
            diffs.insert(mirror.id, Some(convert_file_list_to_diff(files, false)));
            continue;
        }

        let target_pool = get_target_pool(mirror.id.as_str(), Some(&mirror))?
            .ok_or_else(|| format_err!("Failed to open target pool."))?;
        diffs.insert(
            mirror.id,
            Some(source_pool.lock()?.diff_pools(&target_pool)?),
        );
    }

    for dropped in mirror_state.target_only {
        match get_target_pool(&dropped, None)? {
            Some(pool) => {
                let files = pool.lock()?.list_files()?;
                diffs.insert(dropped, Some(convert_file_list_to_diff(files, false)));
            }
            None => {
                diffs.insert(dropped, None);
            }
        }
    }

    Ok(diffs)
}

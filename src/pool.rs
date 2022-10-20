use std::{
    cmp::max,
    collections::{hash_map::Entry, HashMap},
    fs::{hard_link, remove_dir, File, Metadata},
    ops::Deref,
    os::linux::fs::MetadataExt,
    path::{Path, PathBuf},
};

use anyhow::{bail, format_err, Error};
use nix::{unistd, NixPath};

use proxmox_apt::deb822::CheckSums;
use proxmox_sys::fs::{create_path, file_get_contents, replace_file, CreateOptions};
use proxmox_time::epoch_i64;
use walkdir::WalkDir;

use crate::types::Diff;

#[derive(Debug)]
/// Pool consisting of two (possibly overlapping) directory trees:
/// - pool_dir contains checksum files added by `add_file`
/// - link_dir contains directories and hardlinks to checksum files created by `link_file`
///
/// Files are considered orphaned and eligible for GC if they either only exist in pool_dir
/// or only exist in link dir.
pub(crate) struct Pool {
    pool_dir: PathBuf,
    link_dir: PathBuf,
}

/// Lock guard used to guard against concurrent modification
pub(crate) struct PoolLockGuard<'lock> {
    pool: &'lock Pool,
    _lock: Option<File>,
}

impl Pool {
    /// Create a new pool by creating `pool_dir` and `link_dir`.
    ///
    /// Pool dir can already exist, link dir must not exist before calling this function.
    pub(crate) fn create(link_dir: &Path, pool: &Path) -> Result<Self, Error> {
        if link_dir.exists() {
            bail!("Pool link dir {link_dir:?} already exists.");
        }

        if !pool.exists() {
            create_path(pool, None, None)?;
        }

        create_path(link_dir, None, None)?;

        Ok(Self {
            pool_dir: pool.to_path_buf(),
            link_dir: link_dir.to_path_buf(),
        })
    }

    /// Open an existing pool. `pool_dir` and `link_dir` must exist.
    pub(crate) fn open(link_dir: &Path, pool: &Path) -> Result<Self, Error> {
        if !link_dir.exists() {
            bail!("Pool link dir {link_dir:?} doesn't exist.")
        }

        if !pool.exists() {
            bail!("Pool dir {pool:?} doesn't exist.");
        }

        Ok(Self {
            pool_dir: pool.to_path_buf(),
            link_dir: link_dir.to_path_buf(),
        })
    }

    /// Lock a pool to add/remove files or links, or protect against concurrent modifications.
    pub(crate) fn lock(&self) -> Result<PoolLockGuard, Error> {
        let timeout = std::time::Duration::new(30, 0);
        let lock = Some(proxmox_sys::fs::open_file_locked(
            &self.lock_path(),
            timeout,
            true,
            CreateOptions::default(),
        )?);

        Ok(PoolLockGuard {
            pool: self,
            _lock: lock,
        })
    }

    /// Returns whether the pool contain a file for the given checksum.
    pub(crate) fn contains(&self, checksums: &CheckSums) -> bool {
        match self.get_checksum_paths(checksums) {
            Ok(paths) => paths.iter().any(|path| path.exists()),
            Err(_err) => false,
        }
    }

    /// Returns the file contents for a given checksum, optionally `verify`ing whether the on-disk
    /// data matches the checksum.
    pub(crate) fn get_contents(
        &self,
        checksums: &CheckSums,
        verify: bool,
    ) -> Result<Vec<u8>, Error> {
        let source = self
            .get_checksum_paths(checksums)?
            .into_iter()
            .find(|path| path.exists())
            .ok_or_else(|| format_err!("Pool doesn't contain file with this checksum."))?;

        let data = file_get_contents(source)?;
        if verify {
            checksums.verify(&data)?
        };
        Ok(data)
    }

    // Helper to return all possible checksum file paths for a given checksum. Checksums considered
    // insecure will be ignored.
    fn get_checksum_paths(&self, checksums: &CheckSums) -> Result<Vec<PathBuf>, Error> {
        if !checksums.is_secure() {
            bail!("pool cannot operate on files lacking secure checksum!");
        }

        let mut res = Vec::new();

        if let Some(sha512) = checksums.sha512 {
            let mut pool = self.pool_dir.clone();
            pool.push("sha512");
            pool.push(hex::encode(sha512));
            res.push(pool);
        }

        if let Some(sha256) = checksums.sha256 {
            let mut pool = self.pool_dir.clone();
            pool.push("sha256");
            pool.push(hex::encode(sha256));
            res.push(pool);
        }

        if res.is_empty() {
            bail!("Couldn't determine any checksum paths.");
        }

        Ok(res)
    }

    fn path_in_pool(&self, path: &Path) -> bool {
        path.starts_with(&self.pool_dir)
    }

    fn path_in_link_dir(&self, path: &Path) -> bool {
        path.starts_with(&self.link_dir)
    }

    fn lock_path(&self) -> PathBuf {
        let mut lock_path = self.pool_dir.clone();
        lock_path.push(".lock");
        lock_path
    }

    pub(crate) fn get_path(&self, rel_path: &Path) -> Result<PathBuf, Error> {
        let mut path = self.link_dir.clone();
        path.push(rel_path);

        if self.path_in_link_dir(&path) {
            Ok(path)
        } else {
            bail!("Relative path not inside pool's link directory.");
        }
    }
}

impl PoolLockGuard<'_> {
    // Helper to scan the pool for all checksum files and the total link count. The resulting
    // HashMap can be used to check whether files in `link_dir` are properly registered in the
    // pool or orphaned.
    fn get_inode_csum_map(&self) -> Result<(HashMap<u64, CheckSums>, u64), Error> {
        let mut inode_map: HashMap<u64, CheckSums> = HashMap::new();
        let mut link_count = 0;

        for pool_entry in WalkDir::new(&self.pool.pool_dir).into_iter() {
            let pool_entry = pool_entry?;
            let name = pool_entry.file_name().to_owned();

            let path = pool_entry.into_path();
            if path == self.lock_path() {
                continue;
            };

            let meta = path.metadata()?;
            if meta.is_file() {
                let parent_dir_name = path
                    .parent()
                    .and_then(|parent_dir| parent_dir.file_name())
                    .and_then(|dir_name| dir_name.to_str());

                if parent_dir_name.is_none()
                    || (parent_dir_name != Some("sha256") && parent_dir_name != Some("sha512"))
                {
                    eprintln!("skipping unknown pool path {path:?}");
                    continue;
                }

                let csum = match name.len() {
                    128 => {
                        let mut bytes = [0u8; 64];
                        hex::decode_to_slice(name.to_string_lossy().as_bytes(), &mut bytes)?;
                        CheckSums {
                            sha512: Some(bytes),
                            ..Default::default()
                        }
                    }
                    64 => {
                        let mut bytes = [0u8; 32];
                        hex::decode_to_slice(name.to_string_lossy().as_bytes(), &mut bytes)?;
                        CheckSums {
                            sha256: Some(bytes),
                            ..Default::default()
                        }
                    }
                    len => {
                        eprintln!("Invalid checksum file name length {len}: {path:?}");
                        continue;
                    }
                };

                let existing = inode_map.entry(meta.st_ino());

                if let Entry::Vacant(_) = existing {
                    link_count += meta.st_nlink();
                }

                existing.or_default().merge(&csum)?;
            }
        }

        Ok((inode_map, link_count))
    }

    /// Syncs the pool into a target pool, optionally verifying file contents along the way.
    ///
    /// This proceeds in four phases:
    /// - iterate over source pool checksum files to obtain information about existing files
    /// - iterate over source pool links, add missing checksum files and links to target pool
    /// - iterate over target pool links, remove those which are not present in source pool
    /// - if links were removed in phase 3, run GC on target pool
    pub(crate) fn sync_pool(&self, target: &Pool, verify: bool) -> Result<(), Error> {
        let target = target.lock()?;

        let (inode_map, total_link_count) = self.get_inode_csum_map()?;

        let total_count = inode_map.len();
        println!("Found {total_count} pool checksum files.");

        let mut added_count = 0usize;
        let mut added_size = 0usize;
        let mut link_count = 0usize;

        println!("Looking for new files and links..");
        let mut checked_link_count = 0;
        let progress_modulo = max(total_link_count / 50, 10) as usize;
        let mut last_progress = epoch_i64();

        for link_entry in WalkDir::new(&self.pool.link_dir).into_iter() {
            let path = link_entry?.into_path();
            if self.path_in_pool(&path) {
                continue;
            };

            let meta = path.metadata()?;
            if !meta.is_file() {
                continue;
            };

            checked_link_count += 1;

            match inode_map.get(&meta.st_ino()) {
                Some(csum) => {
                    if target.contains(csum) {
                        if verify {
                            target.get_contents(csum, true)?;
                        }
                    } else {
                        let contents = self.get_contents(csum, verify)?;
                        target.add_file(&contents, csum, verify)?;

                        added_count += 1;
                        added_size += contents.len();
                    }

                    let path = path.strip_prefix(&self.pool.link_dir)?;

                    if target.link_file(csum, path)? {
                        link_count += 1;
                    }
                }
                None => bail!("Found file not part of source pool: {path:?}"),
            }

            if checked_link_count % progress_modulo == 0 || last_progress + 30 < epoch_i64() {
                last_progress = epoch_i64();
                println!("Progress: checked {checked_link_count} links; added {added_count} files ({added_size}b) / {link_count} links to target pool");
            }
        }
        println!(
            "Stats: checked {checked_link_count} links; added {added_count} files ({added_size}b) / {link_count} links to target pool"
        );

        println!("Looking for vanished files..");
        let mut vanished_count = 0usize;
        let mut orphaned_count: usize = 0usize;
        let (target_inode_map, _target_link_count) = target.get_inode_csum_map()?;

        for link_entry in WalkDir::new(&target.link_dir).into_iter() {
            let path = link_entry?.into_path();
            if target.path_in_pool(&path) {
                continue;
            };

            let meta = path.metadata()?;
            if !meta.is_file() {
                continue;
            };

            let rel_path = path.strip_prefix(&target.pool.link_dir)?;
            if !self.pool.get_path(rel_path)?.exists() {
                match target_inode_map.get(&meta.st_ino()) {
                    Some(_csum) => {
                        target.unlink_file(&path, true)?;
                        vanished_count += 1;
                    }
                    None => {
                        eprintln!("Found path in target pool that is not registered: {path:?}");
                        orphaned_count += 1;
                    }
                }
            }
        }

        if vanished_count > 0 || orphaned_count > 0 {
            if vanished_count > 0 {
                println!("Unlinked {vanished_count} vanished files.");
            }
            if orphaned_count > 0 {
                println!("Found {orphaned_count} orphaned files.");
            }
            println!("Running GC now.");
            let (count, size) = target.gc()?;
            println!("GC removed {count} files, freeing {size}b");
        } else {
            println!("None found.")
        }

        println!("Syncing done: added {added_count} files ({added_size}b) / {link_count} links to target pool");

        Ok(())
    }

    /// Adds a new checksum file.
    ///
    /// If `checksums` contains multiple trusted checksums, they will be linked to the first checksum file.
    pub(crate) fn add_file(
        &self,
        data: &[u8],
        checksums: &CheckSums,
        sync: bool,
    ) -> Result<(), Error> {
        if self.pool.contains(checksums) {
            bail!("Pool already contains file with this checksum.");
        }

        let mut csum_paths = self.pool.get_checksum_paths(checksums)?.into_iter();
        let first = csum_paths
            .next()
            .ok_or_else(|| format_err!("Failed to determine first checksum path"))?;

        ensure_parent_dir_exists(&first)?;
        replace_file(&first, data, CreateOptions::default(), sync)?;
        for target in csum_paths {
            link_file_do(&first, &target)?;
        }

        Ok(())
    }

    /// Links previously added file into `path` (relative to `link_dir`). Missing parent
    /// directories will be created automatically.
    pub(crate) fn link_file(&self, checksums: &CheckSums, path: &Path) -> Result<bool, Error> {
        let path = self.pool.get_path(path)?;
        if !self.pool.path_in_link_dir(&path) {
            bail!(
                "Cannot link file outside of pool - {:?} -> {:?}.",
                self.pool.link_dir,
                path
            );
        }

        let csum_paths = self.pool.get_checksum_paths(checksums)?;

        let source = csum_paths
            .iter()
            .find(|path| path.exists())
            .ok_or_else(|| format_err!("Cannot link file which doesn't exist in pool."))?;

        if !self.pool.path_in_pool(source) {
            bail!("Cannot link to file outside of pool.");
        }

        link_file_do(source, &path)
    }

    /// Unlink a previously linked file at `path` (absolute, must be below `link_dir`). Optionally
    /// remove any parent directories that became empty.
    pub(crate) fn unlink_file(
        &self,
        mut path: &Path,
        remove_empty_parents: bool,
    ) -> Result<(), Error> {
        if !self.pool.path_in_link_dir(path) {
            bail!("Cannot unlink file outside of pool.");
        }

        unistd::unlink(path)?;

        if !remove_empty_parents {
            return Ok(());
        }

        while let Some(parent) = path.parent() {
            path = parent;

            if !self.pool.path_in_link_dir(path) || !path.is_empty() {
                break;
            }

            remove_dir(path)?;
        }

        Ok(())
    }

    /// Remove a directory tree at `path` (absolute, must be below `link_dir`)
    pub(crate) fn remove_dir(&self, path: &Path) -> Result<(), Error> {
        if !self.pool.path_in_link_dir(path) {
            bail!("Cannot unlink file outside of pool.");
        }

        std::fs::remove_dir_all(path)
            .map_err(|err| format_err!("Failed to remove {path:?} - {err}"))
    }

    /// Run a garbage collection, removing
    /// - any checksum files that have no links outside of `pool_dir`
    /// - any files in `link_dir` that have no corresponding checksum files
    pub(crate) fn gc(&self) -> Result<(usize, u64), Error> {
        let (inode_map, _link_count) = self.get_inode_csum_map()?;

        let mut count = 0;
        let mut size = 0;

        let handle_entry = |entry: Result<walkdir::DirEntry, walkdir::Error>,
                            count: &mut usize,
                            size: &mut u64|
         -> Result<(), Error> {
            let path = entry?.into_path();
            if path == self.lock_path() {
                return Ok(());
            }

            let meta = path.metadata()?;
            if !meta.is_file() {
                return Ok(());
            };
            let remove = if let Some(csum) = inode_map.get(&meta.st_ino()) {
                let expected_link_count = self
                    .get_checksum_paths(csum)?
                    .iter()
                    .filter(|path| path.exists())
                    .count();
                let actual_link_count = meta.st_nlink() as usize;

                match actual_link_count.cmp(&expected_link_count) {
                    std::cmp::Ordering::Less => {
                        println!("Something fishy going on with '{path:?}'");
                        false
                    }
                    std::cmp::Ordering::Equal => {
                        // only checksum files remaining
                        println!("Removing '{:?}'", &path);
                        true
                    }
                    std::cmp::Ordering::Greater => {
                        // still has regular links to checksum files
                        false
                    }
                }
            } else {
                println!("Removing orphan: {path:?}");
                true
            };

            if remove {
                *count += 1;
                *size += meta.st_size();
                unistd::unlink(&path)?;
            }
            Ok(())
        };

        WalkDir::new(&self.pool.link_dir)
            .into_iter()
            .try_for_each(|entry| handle_entry(entry, &mut count, &mut size))?;
        WalkDir::new(&self.pool.pool_dir)
            .into_iter()
            .try_for_each(|entry| handle_entry(entry, &mut count, &mut size))?;

        Ok((count, size))
    }

    /// Destroy this pool instance by removing `link_dir` and running a GC. The pool base dir will remain.
    pub(crate) fn destroy(self) -> Result<(), Error> {
        // remove links so GC can pick them up
        std::fs::remove_dir_all(&self.link_dir)?;

        // GC expects the link dir to exist
        create_path(&self.link_dir, None, None)?;
        self.gc()?;

        // now remove the empty one again
        std::fs::remove_dir_all(&self.link_dir)?;
        Ok(())
    }

    /// Rename a link or directory from `from` to `to` (both relative to `link_dir`).
    pub(crate) fn rename(&self, from: &Path, to: &Path) -> Result<(), Error> {
        let mut abs_from = self.link_dir.clone();
        abs_from.push(from);

        let mut abs_to = self.link_dir.clone();
        abs_to.push(to);

        if !self.path_in_link_dir(&abs_from) || !self.path_in_link_dir(&abs_to) {
            bail!("Can only rename within pool..");
        }

        std::fs::rename(&abs_from, &abs_to)
            .map_err(|err| format_err!("Failed to rename {abs_from:?} to {abs_to:?} - {err}"))
    }

    /// Calculate diff between two pool dirs
    pub(crate) fn diff_dirs(&self, path: &Path, other_path: &Path) -> Result<Diff, Error> {
        let mut diff = Diff::default();

        let handle_entry = |entry: Result<walkdir::DirEntry, walkdir::Error>,
                            base: &Path,
                            other_base: &Path,
                            changed: Option<&mut Vec<(PathBuf, u64)>>,
                            missing: &mut Vec<(PathBuf, u64)>|
         -> Result<(), Error> {
            let path = entry?.into_path();

            let meta = path.metadata()?;
            if !meta.is_file() {
                return Ok(());
            };

            let relative = path.strip_prefix(base)?;
            let mut absolute = other_base.to_path_buf();
            absolute.push(relative);
            if absolute.exists() {
                if let Some(changed) = changed {
                    let other_meta = absolute.metadata()?;
                    if other_meta.st_ino() != meta.st_ino() {
                        changed.push((
                            relative.to_path_buf(),
                            meta.st_size().abs_diff(other_meta.st_size()),
                        ));
                    }
                }
            } else {
                missing.push((relative.to_path_buf(), meta.st_size()));
            }

            Ok(())
        };

        let path = self.get_path(path)?;
        let other_path = self.get_path(other_path)?;

        WalkDir::new(&path).into_iter().try_for_each(|entry| {
            handle_entry(
                entry,
                &path,
                &other_path,
                Some(&mut diff.changed.paths),
                &mut diff.removed.paths,
            )
        })?;
        WalkDir::new(&other_path)
            .into_iter()
            .try_for_each(|entry| {
                handle_entry(entry, &other_path, &path, None, &mut diff.added.paths)
            })?;

        Ok(diff)
    }

    /// Calculate diff between two pools
    pub(crate) fn diff_pools(&self, other: &Pool) -> Result<Diff, Error> {
        let mut diff = Diff::default();

        let handle_entry = |entry: Result<walkdir::DirEntry, walkdir::Error>,
                            pool: &Pool,
                            pool_csums: &HashMap<u64, CheckSums>,
                            other_pool: &Pool,
                            other_csums: &HashMap<u64, CheckSums>,
                            changed: Option<&mut Vec<(PathBuf, u64)>>,
                            missing: &mut Vec<(PathBuf, u64)>|
         -> Result<(), Error> {
            let path = entry?.into_path();

            let meta = path.metadata()?;
            if !meta.is_file() {
                return Ok(());
            };

            let base = &pool.link_dir;

            let relative = path.strip_prefix(base)?;
            let absolute = other_pool.get_path(relative)?;
            if absolute.exists() {
                if let Some(changed) = changed {
                    let csum = match pool_csums.get(&meta.st_ino()) {
                        Some(csum) => csum,
                        None => {
                            eprintln!("{path:?} path not registered with pool.");
                            changed.push((relative.to_path_buf(), 0)); // TODO add warning/error field?
                            return Ok(());
                        }
                    };
                    let other_meta = absolute.metadata()?;
                    let other_csum = match other_csums.get(&other_meta.st_ino()) {
                        Some(csum) => csum,
                        None => {
                            eprintln!("{absolute:?} path not registered with pool.");
                            changed.push((relative.to_path_buf(), 0)); // TODO add warning/error field?
                            return Ok(());
                        }
                    };
                    if csum != other_csum {
                        changed.push((
                            relative.to_path_buf(),
                            meta.st_size().abs_diff(other_meta.st_size()),
                        ));
                    }
                }
            } else {
                missing.push((relative.to_path_buf(), meta.st_size()));
            }

            Ok(())
        };

        let other = other.lock()?;
        let (csums, _) = self.get_inode_csum_map()?;
        let (other_csums, _) = other.get_inode_csum_map()?;

        WalkDir::new(&self.link_dir)
            .into_iter()
            .try_for_each(|entry| {
                handle_entry(
                    entry,
                    self,
                    &csums,
                    &other,
                    &other_csums,
                    Some(&mut diff.changed.paths),
                    &mut diff.removed.paths,
                )
            })?;
        WalkDir::new(&other.link_dir)
            .into_iter()
            .try_for_each(|entry| {
                handle_entry(
                    entry,
                    &other,
                    &other_csums,
                    self,
                    &csums,
                    None,
                    &mut diff.added.paths,
                )
            })?;

        Ok(diff)
    }

    pub(crate) fn list_files(&self) -> Result<Vec<(PathBuf, Metadata)>, Error> {
        let mut file_list = Vec::new();
        WalkDir::new(&self.link_dir)
            .into_iter()
            .try_for_each(|entry| -> Result<(), Error> {
                let path = entry?.into_path();
                let meta = path.metadata()?;
                let relative = path.strip_prefix(&self.link_dir)?;

                file_list.push((relative.to_path_buf(), meta));
                Ok(())
            })?;
        Ok(file_list)
    }
}

fn link_file_do(source: &Path, target: &Path) -> Result<bool, Error> {
    ensure_parent_dir_exists(target)?;
    if !source.exists() {
        bail!("Cannot link file that doesn't exist.");
    }

    if target.exists() {
        let source_inode = source.metadata()?.st_ino();
        let target_inode = target.metadata()?.st_ino();
        if source_inode == target_inode {
            return Ok(false);
        } else {
            bail!(
                "Target path {:?} already exists as link to ino#{:?}, unlink first.",
                target,
                target_inode
            );
        }
    }

    hard_link(source, target)
        .map_err(|err| format_err!("Failed to link {:?} at {:?} - {}", source, target, err))?;

    Ok(true)
}
fn ensure_parent_dir_exists(path: &Path) -> Result<(), Error> {
    let parent = path
        .parent()
        .ok_or_else(|| format_err!("Cannot create parent directory of {:?}", path))?;
    create_path(parent, None, None).map(|_| ())
}

impl Deref for PoolLockGuard<'_> {
    type Target = Pool;

    fn deref(&self) -> &Self::Target {
        self.pool
    }
}

//! Proxmox mirroring tool for APT repositories.
//!
//! This library provides the underlying functionality of the `proxmox-offline-mirror` and
//! `proxmox-apt-repo` binaries.
//!
//! It implements the following features:
//! - local storage in a hardlink-based pool
//! - intelligent fetching only those files of a repository that have changed since the last mirroring operation
//! - syncing to external media

use std::{
    fmt::Display,
    ops::{Add, AddAssign},
    path::Path,
};

use anyhow::{format_err, Error};
use medium::MirrorInfo;
use proxmox_apt::repositories::{APTRepository, APTRepositoryFile, APTRepositoryFileType};
use types::Snapshot;

/// Main configuration file containing definitions of mirrors, external media and subscription keys.
pub mod config;
/// Helpers
pub mod helpers;
/// Operations concerning a medium.
pub mod medium;
/// Operations concerning a mirror.
pub mod mirror;
/// Operations concerning subscription keys.
pub mod subscription;

/// Hardlink pool.
pub(crate) mod pool;
/// Various common types
pub mod types;

/// Combination of data and whether it needed to be fetched or was re-used.
struct FetchResult {
    /// Fetched/read data
    data: Vec<u8>,
    /// Number of bytes fetched (0 if re-using pool data)
    fetched: usize,
}

impl FetchResult {
    fn data(self) -> Vec<u8> {
        self.data
    }

    fn data_ref(&self) -> &[u8] {
        &self.data
    }
}

#[derive(Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
/// To keep track of progress and how much data was newly fetched vs. re-used and just linked
struct Progress {
    new: usize,
    new_bytes: usize,
    reused: usize,
}

impl Progress {
    fn new() -> Self {
        Default::default()
    }
    fn update(&mut self, fetch_result: &FetchResult) {
        if fetch_result.fetched > 0 {
            self.new += 1;
            self.new_bytes += fetch_result.fetched;
        } else {
            self.reused += 1;
        }
    }

    fn file_count(&self) -> usize {
        self.new + self.reused
    }
}

impl Add for Progress {
    type Output = Progress;

    fn add(self, rhs: Self) -> Self::Output {
        Progress {
            new: self.new + rhs.new,
            new_bytes: self.new_bytes + rhs.new_bytes,
            reused: self.reused + rhs.reused,
        }
    }
}

impl AddAssign for Progress {
    fn add_assign(&mut self, rhs: Self) {
        self.new += rhs.new;
        self.new_bytes += rhs.new_bytes;
        self.reused += rhs.reused;
    }
}

impl Display for Progress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.new + self.reused;
        let percent = if total == 0 {
            100f64
        } else {
            self.reused as f64 * 100f64 / total as f64
        };

        f.write_fmt(format_args!(
            "{} new files ({}b), re-used {} existing files ({:.2}% re-used)..",
            self.new, self.new_bytes, self.reused, percent
        ))
    }
}

/// Try to parse a line in sources.list format into an `APTRepository`.
pub(crate) fn convert_repo_line(line: String) -> Result<APTRepository, Error> {
    let mut repository = APTRepositoryFile::with_content(line, APTRepositoryFileType::List);
    repository.parse()?;
    Ok(repository.repositories[0].clone())
}

/// Generate a file-based repository line in sources.list format
pub fn generate_repo_file_line(
    medium_base: &Path,
    mirror_id: &str,
    mirror: &MirrorInfo,
    snapshot: &Snapshot,
) -> Result<String, Error> {
    let mut snapshot_path = medium_base.to_path_buf();
    snapshot_path.push(mirror_id);
    snapshot_path.push(snapshot.to_string());
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

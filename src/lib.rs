use std::{
    fmt::Display,
    ops::{Add, AddAssign},
};

use anyhow::Error;
use proxmox_apt::repositories::{APTRepository, APTRepositoryFile, APTRepositoryFileType};

pub mod config;
pub mod helpers;
pub mod medium;
pub mod mirror;
pub mod pool;
pub mod snapshot;
pub mod types;

struct FetchResult {
    data: Vec<u8>,
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

pub fn convert_repo_line(line: String) -> Result<APTRepository, Error> {
    let mut repository = APTRepositoryFile::with_content(line, APTRepositoryFileType::List);
    repository.parse()?;
    Ok(repository.repositories[0].clone())
}

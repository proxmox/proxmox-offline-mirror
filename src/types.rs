use std::{fmt::Display, path::PathBuf, str::FromStr};

use anyhow::Error;
use proxmox_schema::{ApiStringFormat, Schema, StringSchema, api, const_regex};
use proxmox_serde::{forward_deserialize_to_from_str, forward_serialize_to_display};
use proxmox_time::{epoch_i64, epoch_to_rfc3339_utc, parse_rfc3339};

#[rustfmt::skip]
#[macro_export]
// copied from PBS
macro_rules! PROXMOX_SAFE_ID_REGEX_STR { () => { r"(?:[A-Za-z0-9_][A-Za-z0-9._\-]*)" }; }

const_regex! {
    // copied from PBS
    PROXMOX_SAFE_ID_REGEX = concat!(r"^", PROXMOX_SAFE_ID_REGEX_STR!(), r"$");

}
pub const PROXMOX_SAFE_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);

/// Schema for config IDs
pub const MIRROR_ID_SCHEMA: Schema = StringSchema::new("Mirror name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

/// Schema for config IDs
pub const MEDIA_ID_SCHEMA: Schema = StringSchema::new("Medium name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

#[rustfmt::skip]
#[macro_export]
macro_rules! PROXMOX_SUBSCRIPTION_KEY_REGEX_STR { () => { r"(?:pom-|pve\d+[a-z]-|pbs[a-z]-|pmg[a-z]-).*" }; }

const_regex! {
    PROXMOX_SUBSCRIPTION_KEY_REGEX = concat!(r"^", PROXMOX_SUBSCRIPTION_KEY_REGEX_STR!(), r"$");
}
pub const PROXMOX_SUBSCRIPTION_KEY_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SUBSCRIPTION_KEY_REGEX);

pub const PROXMOX_SUBSCRIPTION_KEY_SCHEMA: Schema = StringSchema::new("Subscription key.")
    .format(&PROXMOX_SUBSCRIPTION_KEY_FORMAT)
    .schema();

#[rustfmt::skip]
#[macro_export]
macro_rules! PROXMOX_SERVER_ID_REGEX_STR { () => { r"[a-fA-F0-9]{32}" }; }

const_regex! {
    PROXMOX_SERVER_ID_REGEX = concat!(r"^", PROXMOX_SERVER_ID_REGEX_STR!(), r"$");
}
pub const PROXMOX_SERVER_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SERVER_ID_REGEX);

pub const PROXMOX_SERVER_ID_SCHEMA: Schema = StringSchema::new("Server ID.")
    .format(&PROXMOX_SERVER_ID_FORMAT)
    .schema();

#[rustfmt::skip]
#[macro_export]
macro_rules! SNAPSHOT_RE { () => (r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z") }
const_regex! {
    pub(crate) SNAPSHOT_REGEX = concat!(r"^", SNAPSHOT_RE!() ,r"$");
}

#[api(
    type: String,
    format: &ApiStringFormat::Pattern(&SNAPSHOT_REGEX),
)]
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Eq, Ord)]
/// Mirror snapshot
pub struct Snapshot(i64);

forward_serialize_to_display!(Snapshot);
forward_deserialize_to_from_str!(Snapshot);

impl Snapshot {
    pub fn now() -> Self {
        Self(epoch_i64())
    }
}

impl Display for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let formatted = epoch_to_rfc3339_utc(self.0).map_err(|_| std::fmt::Error)?;
        f.write_str(&formatted)
    }
}

impl FromStr for Snapshot {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(parse_rfc3339(s)?))
    }
}

/// Entries of Diff
#[derive(Default)]
pub struct DiffMember {
    pub paths: Vec<(PathBuf, u64)>,
}

/// Differences between two pools or pool directories
#[derive(Default)]
pub struct Diff {
    pub added: DiffMember,
    pub changed: DiffMember,
    pub removed: DiffMember,
}

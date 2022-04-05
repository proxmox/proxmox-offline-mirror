use proxmox_schema::{const_regex, ApiStringFormat, Schema, StringSchema};

#[rustfmt::skip]
#[macro_export]
// copied from PBS
macro_rules! PROXMOX_SAFE_ID_REGEX_STR { () => { r"(?:[A-Za-z0-9_][A-Za-z0-9._\-]*)" }; }

const_regex! {
    // copied from PBS
    pub PROXMOX_SAFE_ID_REGEX = concat!(r"^", PROXMOX_SAFE_ID_REGEX_STR!(), r"$");

}
pub const PROXMOX_SAFE_ID_FORMAT: ApiStringFormat =
    ApiStringFormat::Pattern(&PROXMOX_SAFE_ID_REGEX);
pub const MIRROR_ID_SCHEMA: Schema = StringSchema::new("Mirror name.")
    .format(&PROXMOX_SAFE_ID_FORMAT)
    .min_length(3)
    .max_length(32)
    .schema();

#[rustfmt::skip]
#[macro_export]
macro_rules! SNAPSHOT_RE { () => (r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}Z") }
const_regex! {
    pub SNAPSHOT_REGEX = concat!(r"^", SNAPSHOT_RE!() ,r"$");
}

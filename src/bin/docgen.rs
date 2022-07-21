use anyhow::{bail, Error};
use proxmox_offline_mirror::config;

use proxmox_section_config::dump_section_config;

fn get_args() -> (String, Vec<String>) {
    let mut args = std::env::args();
    let prefix = args.next().unwrap();
    let prefix = prefix.rsplit('/').next().unwrap().to_string(); // without path
    let args: Vec<String> = args.collect();

    (prefix, args)
}

fn main() -> Result<(), Error> {
    let (_prefix, args) = get_args();

    if args.is_empty() {
        bail!("missing arguments");
    }

    for arg in args.iter() {
        let text = match arg.as_ref() {
            "mirror.cfg" => dump_section_config(&config::CONFIG),
            _ => bail!("docgen: got unknown type"),
        };
        println!("{}", text);
    }

    Ok(())
}
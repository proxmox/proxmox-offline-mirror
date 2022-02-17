use std::{collections::HashMap, io::Read, path::PathBuf};

use anyhow::{bail, Error};

use config::MirrorConfig;
use flate2::bufread::GzDecoder;
use proxmox_apt::{
    deb822::{CompressionType, FileReference, FileReferenceType, PackagesFile, ReleaseFile},
    repositories::{
        APTRepository, APTRepositoryFile, APTRepositoryFileType, APTRepositoryPackageType,
    },
};
use proxmox_sys::fs::{create_path, file_get_contents, replace_file, CreateOptions};

pub mod config;
mod verifier;

/// Parse a single line in sources.list format
pub fn parse_repo(repo: String) -> Result<APTRepository, Error> {
    let mut repo = APTRepositoryFile::with_content(repo, APTRepositoryFileType::List);
    repo.parse()?;
    Ok(repo.repositories[0].clone())
}

fn get_repo_url(repo: &APTRepository, path: &str) -> String {
    let repo_root = format!("{}/dists/{}", repo.uris[0], repo.suites[0]);

    format!("{}/{}", repo_root, path)
}

fn fetch_repo_file(uri: &str, max_size: Option<u64>) -> Result<Vec<u8>, Error> {
    println!("-> GET '{}'..", uri);

    let response = ureq::get(uri).call()?.into_reader();

    let mut content = Vec::new();
    let bytes = response
        .take(max_size.unwrap_or(10_000_000))
        .read_to_end(&mut content)?;
    println!("<- GOT {} bytes", bytes);

    Ok(content)
}

pub fn fetch_release(
    repo: &APTRepository,
    key: &[u8],
    output_dir: Option<&PathBuf>,
    detached: bool,
) -> Result<ReleaseFile, Error> {
    let (name, content, sig) = if detached {
        println!("Fetching Release/Release.gpg files");
        let sig = fetch_repo_file(&get_repo_url(repo, "Release.gpg"), None)?;
        let content = fetch_repo_file(&get_repo_url(repo, "Release"), Some(32_000_000))?;
        ("Release(.gpg)", content, Some(sig))
    } else {
        println!("Fetching InRelease file");
        let content = fetch_repo_file(&get_repo_url(repo, "InRelease"), Some(32_000_000))?;
        ("InRelease", content, None)
    };

    println!("Verifying '{name}' signature using provided repository key..");
    let verified = verifier::verify_signature(&content[..], key, sig.as_deref())?;
    println!("Success");

    println!("Parsing '{name}'..");
    let parsed: ReleaseFile = verified[..].try_into()?;
    println!(
        "'{name}' file has {} referenced files..",
        parsed.files.len()
    );

    if let Some(output_dir) = output_dir {
        if detached {
            let mut release_file = output_dir.clone();
            release_file.push("Release");
            replace_file(release_file, &content, CreateOptions::default(), true)?;
            let mut release_sig = output_dir.clone();
            release_sig.push("Release.gpg");
            replace_file(release_sig, &sig.unwrap(), CreateOptions::default(), true)?;
        } else {
            let mut in_release = output_dir.clone();
            in_release.push("InRelease");
            replace_file(in_release, &content, CreateOptions::default(), true)?;
        }
    }

    Ok(parsed)
}

pub fn fetch_referenced_file(
    repo: &APTRepository,
    output_dir: Option<&PathBuf>,
    reference: &FileReference,
) -> Result<Vec<u8>, Error> {
    let mut output = None;
    let existing = if let Some(output_dir) = output_dir {
        let mut path = output_dir.clone();
        path.push(&reference.path);
        create_path(&path.parent().unwrap(), None, None)?;
        output = Some(path.clone());

        if let Ok(raw) = file_get_contents(&path) {
            if let Ok(()) = reference.checksums.verify(&raw) {
                output = None;
                Some(raw)
            } else {
                None
            }
        } else {
            None
        }
    } else {
        None
    };

    let raw = if let Some(existing) = existing {
        println!("Reused existing file '{}'", reference.path);
        existing
    } else {
        let new = fetch_repo_file(&get_repo_url(repo, &reference.path), Some(100_000_000))?;
        reference.checksums.verify(&new)?;
        new
    };
    let mut buf = Vec::new();

    let decompressed = match reference.file_type.compression() {
        None => &raw[..],
        Some(CompressionType::Gzip) => {
            let mut gz = GzDecoder::new(&raw[..]);
            gz.read_to_end(&mut buf)?;
            &buf[..]
        }
        Some(CompressionType::Bzip2) => {
            let mut bz = bzip2::read::BzDecoder::new(&raw[..]);
            bz.read_to_end(&mut buf)?;
            &buf[..]
        }
        Some(CompressionType::Lzma) | Some(CompressionType::Xz) => {
            let mut xz = xz2::read::XzDecoder::new(&raw[..]);
            xz.read_to_end(&mut buf)?;
            &buf[..]
        }
    };

    if let Some(path) = output {
        replace_file(path, &raw[..], CreateOptions::default(), true)?;
    }

    Ok(decompressed.to_owned())
}

pub fn mirror(config: &MirrorConfig) -> Result<(), Error> {
    let repo = &config.repository;
    let output_dir = PathBuf::from(&config.path);

    if !output_dir.exists() {
        proxmox_sys::fs::create_dir(&output_dir, CreateOptions::default())?;
    }

    let release = fetch_release(repo, &config.key[..], Some(&output_dir), true)?;
    let _release2 = fetch_release(repo, &config.key[..], Some(&output_dir), false)?;

    let mut per_component = HashMap::new();
    let mut others = Vec::new();
    let binary = repo.types.contains(&APTRepositoryPackageType::Deb);
    let source = repo.types.contains(&APTRepositoryPackageType::DebSrc);

    for (basename, references) in &release.files {
        let reference = references.first();
        let reference = if let Some(reference) = reference {
            reference.clone()
        } else {
            continue;
        };
        let skip_components = !repo.components.contains(&reference.component);

        // TODO make arch filtering some proper thing
        let skip = skip_components
            || match &reference.file_type {
                FileReferenceType::Contents(arch, _)
                | FileReferenceType::ContentsUdeb(arch, _)
                | FileReferenceType::Packages(arch, _) => {
                    !binary || !config.architectures.contains(arch)
                }
                FileReferenceType::Sources(_) => !source,
                _ => false,
            };
        if skip {
            println!("Skipping {}", reference.path);
            others.push(reference);
        } else {
            let list = per_component
                .entry(reference.component)
                .or_insert_with(Vec::new);
            list.push(basename);
        }
    }
    println!();

    let mut indices_size = 0_usize;
    let mut total_count = 0;

    for (component, references) in &per_component {
        println!("Component '{component}'");

        let mut component_indices_size = 0;

        for basename in references {
            for reference in release.files.get(*basename).unwrap() {
                println!("\t{:?}: {:?}", reference.path, reference.file_type);
                component_indices_size += reference.size;
            }
        }
        indices_size += component_indices_size;

        let component_count = references.len();
        total_count += component_count;

        println!("Component references count: {component_count}");
        println!("Component indices size: {component_indices_size}");
        if references.is_empty() {
            println!("\tNo references found..");
        }
    }
    println!("Total indices count: {total_count}");
    println!("Total indices size: {indices_size}");

    if !others.is_empty() {
        println!("Skipped {} references", others.len());
    }
    println!();

    let mut packages_size = 0_usize;
    for (component, references) in per_component {
        println!("Fetching indices for component '{component}'");
        let mut component_deb_size = 0;
        for basename in &references {
            let mut wrote_decompressed = false;
            for reference in release.files.get(*basename).unwrap() {
                match fetch_referenced_file(repo, Some(&output_dir), reference) {
                    Ok(data) => {
                        if !wrote_decompressed {
                            let mut path = output_dir.clone();
                            path.push(basename);
                            replace_file(path, &data[..], CreateOptions::default(), true)?;
                            wrote_decompressed = true;
                        }
                        if matches!(
                            reference.file_type,
                            FileReferenceType::Packages(_, Some(CompressionType::Gzip))
                        ) {
                            let packages: PackagesFile = data[..].try_into()?;
                            let size: usize = packages.files.iter().map(|p| p.size).sum();
                            println!("\t{} packages totalling {size}", packages.files.len());
                            component_deb_size += size;
                        }
                    }
                    Err(err) => {
                        eprintln!("Failed to fetch {} - {}", reference.path, err);
                    }
                };
            }
            if !wrote_decompressed {
                bail!("Failed to write raw file..");
            }
        }
        println!("Total deb size for component: {component_deb_size}");
        packages_size += component_deb_size;
    }
    println!("Total deb size: {packages_size}");

    Ok(())
}

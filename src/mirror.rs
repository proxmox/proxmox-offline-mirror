use std::{
    cmp::max,
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, format_err, Error};
use flate2::bufread::GzDecoder;
use nix::libc;
use proxmox_sys::fs::file_get_contents;
use ureq::Agent;

use crate::{
    config::{MirrorConfig, SubscriptionKey},
    convert_repo_line,
    pool::Pool,
    types::{Snapshot, SNAPSHOT_REGEX},
    FetchResult, Progress,
};
use proxmox_apt::{
    deb822::{
        CheckSums, CompressionType, FileReference, FileReferenceType, PackagesFile, ReleaseFile,
    },
    repositories::{APTRepository, APTRepositoryPackageType},
};

use crate::helpers;

pub(crate) fn pool(config: &MirrorConfig) -> Result<Pool, Error> {
    let pool_dir = format!("{}/.pool", config.dir);
    Pool::open(Path::new(&config.dir), Path::new(&pool_dir))
}

/// `MirrorConfig`, but some fields converted/parsed into usable types.
struct ParsedMirrorConfig {
    pub repository: APTRepository,
    pub architectures: Vec<String>,
    pub pool: Pool,
    pub key: Vec<u8>,
    pub verify: bool,
    pub sync: bool,
    pub auth: Option<String>,
    pub agent: Agent,
}

impl TryInto<ParsedMirrorConfig> for MirrorConfig {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ParsedMirrorConfig, Self::Error> {
        let pool = pool(&self)?;

        let repository = convert_repo_line(self.repository.clone())?;

        let key = file_get_contents(Path::new(&self.key_path))?;

        let agent = ureq::builder()
            .user_agent("proxmox-offline-mirror 0.1") // TODO actually read version ;)
            .build();

        Ok(ParsedMirrorConfig {
            repository,
            architectures: self.architectures,
            pool,
            key,
            verify: self.verify,
            sync: self.sync,
            auth: None,
            agent,
        })
    }
}

// Helper to get absolute URL for dist-specific relative `path`.
fn get_dist_url(repo: &APTRepository, path: &str) -> String {
    let dist_root = format!("{}/dists/{}", repo.uris[0], repo.suites[0]);

    format!("{}/{}", dist_root, path)
}

// Helper to get dist-specific path given a `prefix` (snapshot dir) and relative `path`.
fn get_dist_path(repo: &APTRepository, prefix: &Path, path: &str) -> PathBuf {
    let mut base = PathBuf::from(prefix);
    base.push("dists");
    base.push(&repo.suites[0]);
    base.push(path);
    base
}

// Helper to get generic URL given a `repo` and `path`.
fn get_repo_url(repo: &APTRepository, path: &str) -> String {
    format!("{}/{}", repo.uris[0], path)
}

/// Helper to fetch file from URI and optionally verify the responses checksum.
///
/// Only fetches and returns data, doesn't store anything anywhere.
fn fetch_repo_file(
    agent: &Agent,
    uri: &str,
    max_size: Option<u64>,
    checksums: Option<&CheckSums>,
    auth: Option<&str>,
) -> Result<FetchResult, Error> {
    println!("-> GET '{}'..", uri);

    let request = if let Some(auth) = auth {
        agent.get(uri).set("Authorization", auth)
    } else {
        agent.get(uri)
    };

    let response = request.call()?.into_reader();

    let mut data = Vec::new();
    let bytes = response
        .take(max_size.unwrap_or(10_000_000))
        .read_to_end(&mut data)?;

    if let Some(checksums) = checksums {
        checksums.verify(&data)?;
    }

    Ok(FetchResult {
        data,
        fetched: bytes,
    })
}

/// Helper to fetch InRelease (`detached` == false) or Release/Release.gpg (`detached` == true) files from repository.
///
/// Verifies the contained/detached signature, stores all fetched files under `prefix`, and returns the verified raw release file data.
fn fetch_release(
    config: &ParsedMirrorConfig,
    prefix: &Path,
    detached: bool,
) -> Result<FetchResult, Error> {
    let (name, fetched, sig) = if detached {
        println!("Fetching Release/Release.gpg files");
        let sig = fetch_repo_file(
            &config.agent,
            &get_dist_url(&config.repository, "Release.gpg"),
            None,
            None,
            config.auth.as_deref(),
        )?;
        let mut fetched = fetch_repo_file(
            &config.agent,
            &get_dist_url(&config.repository, "Release"),
            Some(32_000_000),
            None,
            config.auth.as_deref(),
        )?;
        fetched.fetched += sig.fetched;
        ("Release(.gpg)", fetched, Some(sig.data()))
    } else {
        println!("Fetching InRelease file");
        let fetched = fetch_repo_file(
            &config.agent,
            &get_dist_url(&config.repository, "InRelease"),
            Some(32_000_000),
            None,
            config.auth.as_deref(),
        )?;
        ("InRelease", fetched, None)
    };

    println!("Verifying '{name}' signature using provided repository key..");
    let content = fetched.data_ref();
    let verified = helpers::verify_signature(content, &config.key, sig.as_deref())?;
    println!("Success");

    let sha512 = Some(openssl::sha::sha512(content));
    let csums = CheckSums {
        sha512,
        ..Default::default()
    };

    let locked = &config.pool.lock()?;

    if !locked.contains(&csums) {
        locked.add_file(content, &csums, config.sync)?;
    }

    if detached {
        locked.link_file(
            &csums,
            Path::new(&get_dist_path(&config.repository, prefix, "Release")),
        )?;
        let sig = sig.unwrap();
        let sha512 = Some(openssl::sha::sha512(&sig));
        let csums = CheckSums {
            sha512,
            ..Default::default()
        };
        if !locked.contains(&csums) {
            locked.add_file(&sig, &csums, config.sync)?;
        }
        locked.link_file(
            &csums,
            Path::new(&get_dist_path(&config.repository, prefix, "Release.gpg")),
        )?;
    } else {
        locked.link_file(
            &csums,
            Path::new(&get_dist_path(&config.repository, prefix, "InRelease")),
        )?;
    }

    Ok(FetchResult {
        data: verified,
        fetched: fetched.fetched,
    })
}

/// Helper to fetch an index file referenced by a `ReleaseFile`.
///
/// Since these usually come in compressed and uncompressed form, with the latter often not actually existing in the source repository as file, this fetches and if necessary decompresses to obtain a copy of the uncompressed data.
/// Will skip fetching if both references are already available with the expected checksum in the pool, in which case they will just be re-linked under the new path.
///
/// Returns the uncompressed data.
fn fetch_index_file(
    config: &ParsedMirrorConfig,
    prefix: &Path,
    reference: &FileReference,
    uncompressed: &FileReference,
) -> Result<FetchResult, Error> {
    let url = get_dist_url(&config.repository, &reference.path);
    let path = get_dist_path(&config.repository, prefix, &reference.path);
    let uncompressed_path = get_dist_path(&config.repository, prefix, &uncompressed.path);

    if config.pool.contains(&reference.checksums) && config.pool.contains(&uncompressed.checksums) {
        let data = config
            .pool
            .get_contents(&uncompressed.checksums, config.verify)?;

        // Ensure they're linked at current path
        config.pool.lock()?.link_file(&reference.checksums, &path)?;
        config
            .pool
            .lock()?
            .link_file(&uncompressed.checksums, &uncompressed_path)?;
        return Ok(FetchResult { data, fetched: 0 });
    }

    let res = fetch_plain_file(config, &url, &path, &reference.checksums, true)?;

    let mut buf = Vec::new();
    let raw = res.data_ref();

    let decompressed = match reference.file_type.compression() {
        None => raw,
        Some(CompressionType::Gzip) => {
            let mut gz = GzDecoder::new(raw);
            gz.read_to_end(&mut buf)?;
            &buf[..]
        }
        Some(CompressionType::Bzip2) => {
            let mut bz = bzip2::read::BzDecoder::new(raw);
            bz.read_to_end(&mut buf)?;
            &buf[..]
        }
        Some(CompressionType::Lzma) | Some(CompressionType::Xz) => {
            let mut xz = xz2::read::XzDecoder::new(raw);
            xz.read_to_end(&mut buf)?;
            &buf[..]
        }
    };

    let locked = &config.pool.lock()?;
    if !locked.contains(&uncompressed.checksums) {
        locked.add_file(decompressed, &uncompressed.checksums, config.sync)?;
    }

    // Ensure it's linked at current path
    locked.link_file(&uncompressed.checksums, &uncompressed_path)?;

    Ok(FetchResult {
        data: decompressed.to_owned(),
        fetched: res.fetched,
    })
}

/// Helper to fetch arbitrary files like binary packages.
///
/// Will skip fetching if matching file already exists locally, in which case it will just be re-linked under the new path.
///
/// If need_data is false and the mirror config is set to skip verification, reading the file's content will be skipped as well if fetching was skipped.
fn fetch_plain_file(
    config: &ParsedMirrorConfig,
    url: &str,
    file: &Path,
    checksums: &CheckSums,
    need_data: bool,
) -> Result<FetchResult, Error> {
    let locked = &config.pool.lock()?;
    let res = if locked.contains(checksums) {
        if need_data || config.verify {
            locked
                .get_contents(checksums, config.verify)
                .map(|data| FetchResult { data, fetched: 0 })?
        } else {
            // performance optimization for .deb files if verify is false
            // we never need the file contents and they make up the bulk of a repo
            FetchResult {
                data: vec![],
                fetched: 0,
            }
        }
    } else {
        let fetched = fetch_repo_file(
            &config.agent,
            url,
            Some(5_000_000_000),
            Some(checksums),
            config.auth.as_deref(),
        )?;
        locked.add_file(fetched.data_ref(), checksums, config.verify)?;
        fetched
    };

    // Ensure it's linked at current path
    locked.link_file(checksums, file)?;

    Ok(res)
}

/// Initialize a new mirror (by creating the corresponding pool).
pub fn init(config: &MirrorConfig) -> Result<(), Error> {
    let pool_dir = format!("{}/.pool", config.dir);
    Pool::create(Path::new(&config.dir), Path::new(&pool_dir))?;
    Ok(())
}

/// Destroy a mirror (by destroying the corresponding pool).
pub fn destroy(config: &MirrorConfig) -> Result<(), Error> {
    let pool: Pool = pool(config)?;
    pool.lock()?.destroy()?;

    Ok(())
}

/// List snapshots
pub fn list_snapshots(config: &MirrorConfig) -> Result<Vec<Snapshot>, Error> {
    let _pool: Pool = pool(config)?;

    let mut list: Vec<Snapshot> = vec![];

    let path = Path::new(&config.dir);

    proxmox_sys::fs::scandir(
        libc::AT_FDCWD,
        path,
        &SNAPSHOT_REGEX,
        |_l2_fd, snapshot, file_type| {
            if file_type != nix::dir::Type::Directory {
                return Ok(());
            }

            list.push(snapshot.parse()?);

            Ok(())
        },
    )?;

    list.sort_unstable();

    Ok(list)
}

/// Create a new snapshot of the remote repository, fetching and storing files as needed.
///
/// Operates in three phases:
/// - Fetch and verify release files
/// - Fetch referenced indices according to config
/// - Fetch binary packages referenced by package indices
///
/// Files will be linked in a temporary directory and only renamed to the final, valid snapshot directory at the end. In case of error, leftover `XXX.tmp` directories at the top level of `base_dir` can be safely removed once the next snapshot was successfully created, as they only contain hardlinks.
pub fn create_snapshot(
    config: MirrorConfig,
    snapshot: &Snapshot,
    subscription: Option<SubscriptionKey>,
) -> Result<(), Error> {
    let auth = if let Some(product) = &config.use_subscription {
        match subscription {
            None => {
                bail!(
                    "Mirror {} requires a subscription key, but none given.",
                    config.id
                );
            }
            Some(key) if key.product() == *product => {
                let base64 = base64::encode(format!("{}:{}", key.key, key.server_id));
                Some(format!("basic {base64}"))
            }
            Some(key) => {
                bail!(
                    "Repository product type '{}' and key product type '{}' don't match.",
                    product,
                    key.product()
                );
            }
        }
    } else {
        None
    };

    let mut config: ParsedMirrorConfig = config.try_into()?;
    config.auth = auth;

    let prefix = format!("{snapshot}.tmp");
    let prefix = Path::new(&prefix);

    let mut total_progress = Progress::new();

    let parse_release = |res: FetchResult, name: &str| -> Result<ReleaseFile, Error> {
        println!("Parsing {name}..");
        let parsed: ReleaseFile = res.data[..].try_into()?;
        println!(
            "'{name}' file has {} referenced files..",
            parsed.files.len()
        );
        Ok(parsed)
    };

    // we want both on-disk for compat reasons
    let res = fetch_release(&config, prefix, true)?;
    total_progress.update(&res);
    let _release = parse_release(res, "Release")?;

    let res = fetch_release(&config, prefix, false)?;
    total_progress.update(&res);
    let release = parse_release(res, "InRelease")?;

    let mut per_component = HashMap::new();
    let mut others = Vec::new();
    let binary = &config
        .repository
        .types
        .contains(&APTRepositoryPackageType::Deb);
    let source = &config
        .repository
        .types
        .contains(&APTRepositoryPackageType::DebSrc);

    for (basename, references) in &release.files {
        let reference = references.first();
        let reference = if let Some(reference) = reference {
            reference.clone()
        } else {
            continue;
        };
        let skip_components = !&config.repository.components.contains(&reference.component);

        let skip = skip_components
            || match &reference.file_type {
                FileReferenceType::Ignored => true,
                FileReferenceType::PDiff => true, // would require fetching the patches as well
                FileReferenceType::Contents(arch, _)
                | FileReferenceType::ContentsUdeb(arch, _)
                | FileReferenceType::Packages(arch, _)
                | FileReferenceType::PseudoRelease(Some(arch)) => {
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
    let mut packages_indices = HashMap::new();
    for (component, references) in per_component {
        println!("\nFetching indices for component '{component}'");
        let mut component_deb_size = 0;
        let mut fetch_progress = Progress::new();

        for basename in references {
            println!("\tFetching '{basename}'..");
            let files = release.files.get(basename).unwrap();
            let uncompressed_ref = files
                .iter()
                .find(|reference| reference.path == *basename)
                .ok_or_else(|| format_err!("Found derived reference without base reference."))?;
            let mut package_index_data = None;

            for reference in files {
                // if both compressed and uncompressed are referenced, the uncompressed file may not exist on the server
                if reference == uncompressed_ref && files.len() > 1 {
                    continue;
                }

                // this will ensure the uncompressed file will be written locally
                let res = fetch_index_file(&config, prefix, reference, uncompressed_ref)?;
                fetch_progress.update(&res);

                if package_index_data.is_none() && reference.file_type.is_package_index() {
                    package_index_data = Some(res.data());
                }
            }
            if let Some(data) = package_index_data {
                let packages: PackagesFile = data[..].try_into()?;
                let size: usize = packages.files.iter().map(|p| p.size).sum();
                println!("\t{} packages totalling {size}", packages.files.len());
                component_deb_size += size;

                packages_indices.entry(basename).or_insert(packages);
            }
            println!("Progress: {fetch_progress}");
        }
        println!("Total deb size for component: {component_deb_size}");
        packages_size += component_deb_size;
        total_progress += fetch_progress;
    }
    println!("Total deb size: {packages_size}");

    println!("\nFetching packages..");
    for (basename, references) in packages_indices {
        let total_files = references.files.len();
        if total_files == 0 {
            println!("\n{basename} - no files, skipping.");
            continue;
        } else {
            println!("\n{basename} - {total_files} total file(s)");
        }

        let mut fetch_progress = Progress::new();
        for package in references.files {
            let mut full_path = PathBuf::from(prefix);
            full_path.push(&package.file);
            let res = fetch_plain_file(
                &config,
                &get_repo_url(&config.repository, &package.file),
                &full_path,
                &package.checksums,
                false,
            )?;
            fetch_progress.update(&res);
            if fetch_progress.file_count() % (max(total_files / 100, 1)) == 0 {
                println!("\tProgress: {fetch_progress}");
            }
        }
        println!("\tProgress: {fetch_progress}");
        total_progress += fetch_progress;
    }

    println!("\nStats: {total_progress}");

    println!("Rotating temp. snapshot in-place: {prefix:?} -> \"{snapshot}\"");
    let locked = config.pool.lock()?;
    locked.rename(prefix, Path::new(&format!("{snapshot}")))?;

    Ok(())
}

/// Remove a snapshot by removing the corresponding snapshot directory. To actually free up space, a garbage collection needs to be run afterwards.
pub fn remove_snapshot(config: &MirrorConfig, snapshot: &Snapshot) -> Result<(), Error> {
    let pool: Pool = pool(config)?;
    let path = pool.get_path(Path::new(&snapshot.to_string()))?;

    pool.lock()?.remove_dir(&path)
}

/// Run a garbage collection on the underlying pool.
pub fn gc(config: &MirrorConfig) -> Result<(usize, u64), Error> {
    let pool: Pool = pool(config)?;

    pool.lock()?.gc()
}

use std::{
    cmp::max,
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{bail, format_err, Error};
use flate2::bufread::GzDecoder;
use nix::libc;
use proxmox_http::{client::sync::Client, HttpClient, HttpOptions};
use proxmox_sys::fs::file_get_contents;

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

fn mirror_dir(config: &MirrorConfig) -> String {
    format!("{}/{}", config.base_dir, config.id)
}

pub(crate) fn pool(config: &MirrorConfig) -> Result<Pool, Error> {
    let pool_dir = format!("{}/.pool", config.base_dir);
    Pool::open(Path::new(&mirror_dir(config)), Path::new(&pool_dir))
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
    pub client: Client,
    pub ignore_errors: bool,
}

impl TryInto<ParsedMirrorConfig> for MirrorConfig {
    type Error = anyhow::Error;

    fn try_into(self) -> Result<ParsedMirrorConfig, Self::Error> {
        let pool = pool(&self)?;

        let repository = convert_repo_line(self.repository.clone())?;

        let key = file_get_contents(Path::new(&self.key_path))?;

        let options = HttpOptions {
            user_agent: Some("proxmox-offline-mirror 0.1".to_string()),
            ..Default::default()
        }; // TODO actually read version ;)

        let client = Client::new(options);

        Ok(ParsedMirrorConfig {
            repository,
            architectures: self.architectures,
            pool,
            key,
            verify: self.verify,
            sync: self.sync,
            auth: None,
            client,
            ignore_errors: self.ignore_errors,
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
    client: &Client,
    uri: &str,
    max_size: usize,
    checksums: Option<&CheckSums>,
    auth: Option<&str>,
) -> Result<FetchResult, Error> {
    println!("-> GET '{}'..", uri);

    let headers = if let Some(auth) = auth {
        let mut map = HashMap::new();
        map.insert("Authorization".to_string(), auth.to_string());
        Some(map)
    } else {
        None
    };

    let response = client.get(uri, headers.as_ref())?;

    let reader: Box<dyn Read> = response.into_body();
    let mut reader = reader.take(max_size as u64);
    let mut data = Vec::new();
    reader.read_to_end(&mut data)?;

    if let Some(checksums) = checksums {
        checksums.verify(&data)?;
    }

    Ok(FetchResult {
        fetched: data.len(),
        data,
    })
}

/// Helper to fetch InRelease (`detached` == false) or Release/Release.gpg (`detached` == true) files from repository.
///
/// Verifies the contained/detached signature, stores all fetched files under `prefix`, and returns the verified raw release file data.
fn fetch_release(
    config: &ParsedMirrorConfig,
    prefix: &Path,
    detached: bool,
    dry_run: bool,
) -> Result<FetchResult, Error> {
    let (name, fetched, sig) = if detached {
        println!("Fetching Release/Release.gpg files");
        let sig = fetch_repo_file(
            &config.client,
            &get_dist_url(&config.repository, "Release.gpg"),
            1024 * 1024,
            None,
            config.auth.as_deref(),
        )?;
        let mut fetched = fetch_repo_file(
            &config.client,
            &get_dist_url(&config.repository, "Release"),
            256 * 1024 * 1024,
            None,
            config.auth.as_deref(),
        )?;
        fetched.fetched += sig.fetched;
        ("Release(.gpg)", fetched, Some(sig.data()))
    } else {
        println!("Fetching InRelease file");
        let fetched = fetch_repo_file(
            &config.client,
            &get_dist_url(&config.repository, "InRelease"),
            256 * 1024 * 1024,
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

    if dry_run {
        return Ok(FetchResult {
            data: verified,
            fetched: fetched.fetched,
        });
    }

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
    uncompressed: Option<&FileReference>,
    by_hash: bool,
    dry_run: bool,
) -> Result<FetchResult, Error> {
    let url = get_dist_url(&config.repository, &reference.path);
    let path = get_dist_path(&config.repository, prefix, &reference.path);

    if let Some(uncompressed) = uncompressed {
        let uncompressed_path = get_dist_path(&config.repository, prefix, &uncompressed.path);

        if config.pool.contains(&reference.checksums)
            && config.pool.contains(&uncompressed.checksums)
        {
            let data = config
                .pool
                .get_contents(&uncompressed.checksums, config.verify)?;

            if dry_run {
                return Ok(FetchResult { data, fetched: 0 });
            }
            // Ensure they're linked at current path
            config.pool.lock()?.link_file(&reference.checksums, &path)?;
            config
                .pool
                .lock()?
                .link_file(&uncompressed.checksums, &uncompressed_path)?;
            return Ok(FetchResult { data, fetched: 0 });
        }
    }

    let urls = if by_hash {
        let mut urls = Vec::new();
        if let Some((base_url, _file_name)) = url.rsplit_once('/') {
            if let Some(sha512) = reference.checksums.sha512 {
                urls.push(format!("{base_url}/by-hash/SHA512/{}", hex::encode(sha512)));
            }
            if let Some(sha256) = reference.checksums.sha256 {
                urls.push(format!("{base_url}/by-hash/SHA256/{}", hex::encode(sha256)));
            }
        }
        urls.push(url);
        urls
    } else {
        vec![url]
    };

    let res = urls
        .iter()
        .fold(None, |res, url| match res {
            Some(Ok(res)) => Some(Ok(res)),
            _ => Some(fetch_plain_file(
                config,
                url,
                &path,
                reference.size,
                &reference.checksums,
                true,
                dry_run,
            )),
        })
        .ok_or_else(|| format_err!("Failed to retrieve {}", reference.path))??;

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
            let mut xz = xz2::read::XzDecoder::new_multi_decoder(raw);
            xz.read_to_end(&mut buf)?;
            &buf[..]
        }
    };
    let res = FetchResult {
        data: decompressed.to_owned(),
        fetched: res.fetched,
    };

    if dry_run {
        return Ok(res);
    }

    let locked = &config.pool.lock()?;
    if let Some(uncompressed) = uncompressed {
        if !locked.contains(&uncompressed.checksums) {
            locked.add_file(decompressed, &uncompressed.checksums, config.sync)?;
        }

        // Ensure it's linked at current path
        let uncompressed_path = get_dist_path(&config.repository, prefix, &uncompressed.path);
        locked.link_file(&uncompressed.checksums, &uncompressed_path)?;
    }

    Ok(res)
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
    max_size: usize,
    checksums: &CheckSums,
    need_data: bool,
    dry_run: bool,
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
    } else if dry_run && !need_data {
        FetchResult {
            data: vec![],
            fetched: 0,
        }
    } else {
        let fetched = fetch_repo_file(
            &config.client,
            url,
            max_size,
            Some(checksums),
            config.auth.as_deref(),
        )?;
        locked.add_file(fetched.data_ref(), checksums, config.verify)?;
        fetched
    };

    if !dry_run {
        // Ensure it's linked at current path
        locked.link_file(checksums, file)?;
    }

    Ok(res)
}

/// Initialize a new mirror (by creating the corresponding pool).
pub fn init(config: &MirrorConfig) -> Result<(), Error> {
    let pool_dir = format!("{}/.pool", config.base_dir);

    let dir = format!("{}/{}", config.base_dir, config.id);

    Pool::create(Path::new(&dir), Path::new(&pool_dir))?;
    Ok(())
}

/// Destroy a mirror (by destroying the corresponding pool's link dir followed by GC).
pub fn destroy(config: &MirrorConfig) -> Result<(), Error> {
    let pool: Pool = pool(config)?;
    pool.lock()?.destroy()?;

    Ok(())
}

/// List snapshots
pub fn list_snapshots(config: &MirrorConfig) -> Result<Vec<Snapshot>, Error> {
    let _pool: Pool = pool(config)?;

    let mut list: Vec<Snapshot> = vec![];

    let dir = mirror_dir(config);

    let path = Path::new(&dir);

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
    dry_run: bool,
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
    let res = fetch_release(&config, prefix, true, dry_run)?;
    total_progress.update(&res);
    let _release = parse_release(res, "Release")?;

    let res = fetch_release(&config, prefix, false, dry_run)?;
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
                FileReferenceType::Sources(_) => !source,
                _ => {
                    if let Some(arch) = reference.file_type.architecture() {
                        !binary || !config.architectures.contains(arch)
                    } else {
                        false
                    }
                }
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
    let mut failed_references = Vec::new();
    for (component, references) in per_component {
        println!("\nFetching indices for component '{component}'");
        let mut component_deb_size = 0;
        let mut fetch_progress = Progress::new();

        for basename in references {
            println!("\tFetching '{basename}'..");
            let files = release.files.get(basename).unwrap();
            let uncompressed_ref = files.iter().find(|reference| reference.path == *basename);

            let mut package_index_data = None;

            for reference in files {
                // if both compressed and uncompressed are referenced, the uncompressed file may not exist on the server
                if Some(reference) == uncompressed_ref && files.len() > 1 {
                    continue;
                }

                // this will ensure the uncompressed file will be written locally
                let res = match fetch_index_file(
                    &config,
                    prefix,
                    reference,
                    uncompressed_ref,
                    release.aquire_by_hash,
                    dry_run,
                ) {
                    Ok(res) => res,
                    Err(err) if !reference.file_type.is_package_index() => {
                        eprintln!(
                            "Failed to fetch '{:?}' type reference '{}', skipping - {err}",
                            reference.file_type, reference.path
                        );
                        failed_references.push(reference);
                        continue;
                    }
                    Err(err) => bail!(err),
                };
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
    if !failed_references.is_empty() {
        eprintln!("Failed to download non-package-index references:");
        for reference in failed_references {
            eprintln!("\t{}", reference.path);
        }
    }

    println!("\nFetching packages..");
    let mut dry_run_progress = Progress::new();
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
            let url = get_repo_url(&config.repository, &package.file);

            if dry_run {
                if config.pool.contains(&package.checksums) {
                    fetch_progress.update(&FetchResult {
                        data: vec![],
                        fetched: 0,
                    });
                } else {
                    println!("\t(dry-run) GET missing '{url}' ({}b)", package.size);
                    fetch_progress.update(&FetchResult {
                        data: vec![],
                        fetched: package.size,
                    });
                }
            } else {
                let mut full_path = PathBuf::from(prefix);
                full_path.push(&package.file);

                match fetch_plain_file(
                    &config,
                    &url,
                    &full_path,
                    package.size,
                    &package.checksums,
                    false,
                    dry_run,
                ) {
                    Ok(res) => fetch_progress.update(&res),
                    Err(err) if config.ignore_errors => {
                        let msg = format!(
                            "{}: failed to fetch package '{}' - {}",
                            basename, package.file, err,
                        );
                        eprintln!("{msg}");
                    }
                    res => {
                        res?;
                    }
                }
            }

            if fetch_progress.file_count() % (max(total_files / 100, 1)) == 0 {
                println!("\tProgress: {fetch_progress}");
            }
        }
        println!("\tProgress: {fetch_progress}");
        if dry_run {
            dry_run_progress += fetch_progress;
        } else {
            total_progress += fetch_progress;
        }
    }

    if dry_run {
        println!("\nDry-run Stats (indices, downloaded but not persisted):\n{total_progress}");
        println!("\nDry-run stats (packages, new == missing):\n{dry_run_progress}");
    } else {
        println!("\nStats: {total_progress}");
    }

    if !dry_run {
        println!("Rotating temp. snapshot in-place: {prefix:?} -> \"{snapshot}\"");
        let locked = config.pool.lock()?;
        locked.rename(prefix, Path::new(&format!("{snapshot}")))?;
    }

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

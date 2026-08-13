#![forbid(unsafe_code)]

use std::fmt::Write as _;
use std::fs::{self, File};
use std::io::{self, Cursor};
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;

const LATEST_RELEASE_API: &str = "https://api.github.com/repos/broccolai/lotus/releases/latest";
const RELEASE_BASE_URL: &str = "https://github.com/broccolai/lotus/releases";
const DOWNLOAD_LIMIT: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Release {
    pub version: String,
    pub page_url: String,
    archive_url: String,
    checksum_url: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum UpdateStatus {
    Current { release: Release },
    Available { current: String, release: Release },
    Ahead { current: String, release: Release },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StagedUpdate {
    pub version: String,
    pub executable: PathBuf,
    pub directory: PathBuf,
}

#[derive(Debug, Error)]
pub enum UpdateError {
    #[error("Lotus has an invalid current version: {0}")]
    CurrentVersion(#[source] semver::Error),
    #[error("GitHub could not be reached: {0}")]
    Request(#[source] ureq::Error),
    #[error("GitHub returned an invalid Lotus release version: {tag}")]
    ReleaseVersion {
        tag: String,
        #[source]
        source: semver::Error,
    },
    #[error("GitHub returned an invalid release checksum")]
    InvalidChecksum,
    #[error("the downloaded release did not match its published SHA-256 checksum")]
    ChecksumMismatch,
    #[error("the Lotus release archive was invalid: {0}")]
    Archive(#[source] zip::result::ZipError),
    #[error("the Lotus release archive did not contain lotus.exe")]
    MissingExecutable,
    #[error("Lotus could not stage the update: {0}")]
    Staging(#[source] io::Error),
}

pub fn check(current_version: &str) -> Result<UpdateStatus, UpdateError> {
    let current = Version::parse(current_version).map_err(UpdateError::CurrentVersion)?;
    let release = fetch_release()?;
    let latest = Version::parse(&release.version)
        .map_err(|source| UpdateError::ReleaseVersion { tag: release.version.clone(), source })?;
    match latest.cmp(&current) {
        std::cmp::Ordering::Greater => {
            Ok(UpdateStatus::Available { current: current.to_string(), release })
        }
        std::cmp::Ordering::Equal => Ok(UpdateStatus::Current { release }),
        std::cmp::Ordering::Less => {
            Ok(UpdateStatus::Ahead { current: current.to_string(), release })
        }
    }
}

pub fn stage(release: &Release) -> Result<StagedUpdate, UpdateError> {
    let checksum = download(&release.checksum_url)?;
    let expected = std::str::from_utf8(&checksum)
        .ok()
        .and_then(|value| value.split_whitespace().next())
        .filter(|value| value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .ok_or(UpdateError::InvalidChecksum)?;
    let archive = download(&release.archive_url)?;
    let digest = Sha256::digest(&archive);
    let mut actual = String::with_capacity(64);
    for byte in digest {
        write!(actual, "{byte:02x}").expect("writing to a String cannot fail");
    }
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(UpdateError::ChecksumMismatch);
    }

    let directory = staging_directory(&release.version);
    fs::create_dir_all(&directory).map_err(UpdateError::Staging)?;
    let executable = directory.join("lotus.exe");
    let mut archive = zip::ZipArchive::new(Cursor::new(archive)).map_err(UpdateError::Archive)?;
    let mut entry = archive.by_name("lotus.exe").map_err(|error| match error {
        zip::result::ZipError::FileNotFound => UpdateError::MissingExecutable,
        error => UpdateError::Archive(error),
    })?;
    let mut output = File::create(&executable).map_err(UpdateError::Staging)?;
    io::copy(&mut entry, &mut output).map_err(UpdateError::Staging)?;
    output.sync_all().map_err(UpdateError::Staging)?;
    Ok(StagedUpdate { version: release.version.clone(), executable, directory })
}

fn fetch_release() -> Result<Release, UpdateError> {
    let release = agent()
        .get(LATEST_RELEASE_API)
        .header("Accept", "application/vnd.github+json")
        .header("X-GitHub-Api-Version", "2022-11-28")
        .header("User-Agent", concat!("Lotus/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(UpdateError::Request)?
        .body_mut()
        .read_json::<GitHubRelease>()
        .map_err(UpdateError::Request)?;
    let tag = release.tag_name.trim();
    let version = tag.strip_prefix('v').unwrap_or(tag);
    Version::parse(version)
        .map_err(|source| UpdateError::ReleaseVersion { tag: release.tag_name.clone(), source })?;
    let archive_name = format!("lotus-v{version}-windows-x86_64.zip");
    let download_base = format!("{RELEASE_BASE_URL}/download/{}", release.tag_name);
    Ok(Release {
        version: version.to_owned(),
        page_url: format!("{RELEASE_BASE_URL}/tag/{}", release.tag_name),
        archive_url: format!("{download_base}/{archive_name}"),
        checksum_url: format!("{download_base}/{archive_name}.sha256"),
    })
}

fn download(url: &str) -> Result<Vec<u8>, UpdateError> {
    agent()
        .get(url)
        .header("User-Agent", concat!("Lotus/", env!("CARGO_PKG_VERSION")))
        .call()
        .map_err(UpdateError::Request)?
        .body_mut()
        .with_config()
        .limit(DOWNLOAD_LIMIT)
        .read_to_vec()
        .map_err(UpdateError::Request)
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(30))).build().into()
}

fn staging_directory(version: &str) -> PathBuf {
    let nonce =
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |duration| duration.as_nanos());
    std::env::temp_dir().join(format!("lotus-update-{version}-{}-{nonce}", std::process::id()))
}

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
}

// Self-update for the borehole CLI.
//
// Queries the GitHub Releases of the repository, compares semantic versions and
// (for `borehole update`) downloads the matching prebuilt binary and replaces
// the running executable in place. All network calls are blocking (`ureq`), so
// callers in async contexts must use `spawn_blocking`.

use std::io::Write;

use anyhow::{bail, Context, Result};
use owo_colors::OwoColorize;
use semver::Version;
use serde::Deserialize;

use crate::VERSION;

/// Default repository to fetch releases from. Overridable via `BOREHOLE_REPO`.
const DEFAULT_REPO: &str = "llinguini/borehole";

/// User-Agent required by the GitHub API.
const USER_AGENT: &str = concat!("borehole/", env!("BOREHOLE_VERSION"));

/// Upper bound for the downloaded binary, guarding against a malicious server.
const MAX_BINARY_BYTES: u64 = 64 * 1024 * 1024;

/// Minimal projection of the GitHub "latest release" response.
#[derive(Deserialize)]
struct LatestRelease {
    tag_name: String,
}

/// Returns the configured `owner/repo` (overridable via `BOREHOLE_REPO`).
fn repo() -> String {
    std::env::var("BOREHOLE_REPO")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| DEFAULT_REPO.to_string())
}

/// Maps the current OS/arch to the Release asset name, mirroring `install.sh`
/// (Linux x86_64 uses the portable musl build).
fn asset_name() -> Result<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let name = match (os, arch) {
        ("linux", "x86_64") => "borehole-x86_64-unknown-linux-musl",
        ("macos", "x86_64") => "borehole-x86_64-apple-darwin",
        ("macos", "aarch64") => "borehole-aarch64-apple-darwin",
        ("windows", "x86_64") => "borehole-x86_64-pc-windows-msvc.exe",
        _ => bail!("no prebuilt binary for {os}/{arch}; build from source"),
    };
    Ok(name)
}

/// Parses a version string, tolerating a leading `v`.
fn parse_version(s: &str) -> Result<Version> {
    Version::parse(s.trim_start_matches('v'))
        .with_context(|| format!("invalid version string: {s}"))
}

/// Fetches the latest release tag (e.g. "v0.1.2") from the GitHub API.
fn fetch_latest_tag() -> Result<String> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", repo());
    let release: LatestRelease = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .context("failed to query GitHub for the latest release")?
        .body_mut()
        .read_json()
        .context("failed to parse the GitHub release response")?;
    Ok(release.tag_name)
}

/// Best-effort check for a newer release. Returns the newer version (without the
/// leading `v`) when one exists, or `None` when already up to date. Blocking.
pub fn check_newer() -> Result<Option<String>> {
    let current = parse_version(VERSION)?;
    let tag = fetch_latest_tag()?;
    let latest = parse_version(&tag)?;
    Ok((latest > current).then(|| tag.trim_start_matches('v').to_string()))
}

/// Downloads the asset for `tag` and atomically replaces the running binary.
fn download_and_replace(tag: &str) -> Result<()> {
    let asset = asset_name()?;
    let url = format!(
        "https://github.com/{}/releases/download/{}/{}",
        repo(),
        tag,
        asset
    );

    println!("Downloading {asset} ({tag})...");
    let bytes = ureq::get(&url)
        .header("User-Agent", USER_AGENT)
        .call()
        .with_context(|| format!("failed to download {url}"))?
        .body_mut()
        .with_config()
        .limit(MAX_BINARY_BYTES)
        .read_to_vec()
        .context("failed to read the downloaded binary")?;

    // Stage the new binary next to the current executable (same filesystem, so
    // the in-place replacement avoids a cross-device copy where possible).
    let current_exe = std::env::current_exe().context("cannot locate current executable")?;
    let dir = current_exe
        .parent()
        .context("current executable has no parent directory")?;

    let mut tmp = tempfile::Builder::new()
        .prefix(".borehole-update-")
        .tempfile_in(dir)
        .context("failed to create a temporary file for the update")?;
    tmp.write_all(&bytes)
        .context("failed to write the downloaded binary")?;
    tmp.flush().ok();

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = tmp.as_file().metadata()?.permissions();
        perms.set_mode(0o755);
        tmp.as_file().set_permissions(perms)?;
    }

    let (_, tmp_path) = tmp.keep().context("failed to persist the temporary binary")?;
    let result = self_replace::self_replace(&tmp_path)
        .context("failed to replace the running executable");
    // Best-effort cleanup; the swap may already have consumed the temp file.
    let _ = std::fs::remove_file(&tmp_path);
    result
}

/// Runs `borehole update`: replaces the CLI with the latest release if newer.
/// Blocking; intended to run on a `spawn_blocking` task.
pub fn run() -> Result<()> {
    println!("Current version: v{VERSION}");

    let tag = fetch_latest_tag().context("could not determine the latest version")?;
    let latest = parse_version(&tag)?;
    let current = parse_version(VERSION)?;

    if latest <= current {
        println!("{} Already up to date.", "✓".green());
        return Ok(());
    }

    println!("Updating to {tag}...");
    download_and_replace(&tag)?;
    println!(
        "{} Updated to {tag}. Run `borehole --version` to confirm.",
        "✓".green()
    );
    Ok(())
}

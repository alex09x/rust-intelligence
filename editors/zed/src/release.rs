/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx` and `ridx-driver` from the latest GitHub release, in a directory per version inside the extension's own
//! directory; older versions are removed. Zed's rule for language server extensions: download the server or find it,
//! never bundle it. The Rust toolchain `ridx-driver` runs inside is the user's to install: `ridx` names the command
//! when it is missing.

use std::fs;
use zed_extension_api::{self as zed, LanguageServerId, LanguageServerInstallationStatus as Status, Result};

const REPO: &str = "alex09x/rust-intelligence";

/// The release target of a platform, and its archive's extension and kind; `None` where no binary is published.
fn target(os: zed::Os, arch: zed::Architecture) -> Option<(&'static str, &'static str, zed::DownloadedFileType)> {
    use zed::{Architecture::*, DownloadedFileType::*, Os::*};
    Some(match (os, arch) {
        (Mac, Aarch64) => ("aarch64-apple-darwin", "tar.gz", GzipTar),
        (Mac, X8664) => ("x86_64-apple-darwin", "tar.gz", GzipTar),
        (Linux, Aarch64) => ("aarch64-unknown-linux-gnu", "tar.gz", GzipTar),
        (Linux, X8664) => ("x86_64-unknown-linux-gnu", "tar.gz", GzipTar),
        (Windows, X8664) => ("x86_64-pc-windows-msvc", "zip", Zip),
        _ => return None,
    })
}

const NO_BINARY: &str = "no ridx release binary for this platform: install ridx and ridx-driver with cargo \
     (https://github.com/alex09x/rust-intelligence#install) or set lsp.rust-intelligence.binary.path";

/// The path of a released `ridx` for this platform, downloaded if this version is not here yet.
pub fn ridx(server: &LanguageServerId, downloaded: &mut Option<String>) -> Result<String> {
    if let Some(path) = downloaded.as_ref().filter(|p| fs::metadata(p).is_ok_and(|m| m.is_file())) {
        return Ok(path.clone());
    }
    let result = fetch(server);
    let status = match &result {
        Ok(_) => Status::None,
        Err(e) => Status::Failed(e.clone()),
    };
    zed::set_language_server_installation_status(server, &status);
    *downloaded = Some(result?);
    Ok(downloaded.clone().unwrap_or_default())
}

fn fetch(server: &LanguageServerId) -> Result<String> {
    let (os, arch) = zed::current_platform();
    let (target, extension, kind) = target(os, arch).ok_or(NO_BINARY)?;
    zed::set_language_server_installation_status(server, &Status::CheckingForUpdate);
    let release = zed::latest_github_release(REPO, zed::GithubReleaseOptions { require_assets: true, pre_release: false })?;
    let archive = format!("ridx-{target}.{extension}");
    let asset = release.assets.iter().find(|a| a.name == archive).ok_or_else(|| format!("{} has no {archive}: {NO_BINARY}", release.version))?;
    let exe = if os == zed::Os::Windows { ".exe" } else { "" };
    let dir = format!("ridx-{}", release.version);
    let binary = format!("{dir}/ridx-{target}/ridx{exe}");
    if !fs::metadata(&binary).is_ok_and(|m| m.is_file()) {
        zed::set_language_server_installation_status(server, &Status::Downloading);
        zed::download_file(&asset.download_url, &dir, kind).map_err(|e| format!("downloading {archive}: {e}"))?;
        for name in ["ridx", "ridx-driver"] {
            zed::make_file_executable(&format!("{dir}/ridx-{target}/{name}{exe}"))?;
        }
        remove_other_versions(&dir);
    }
    Ok(binary)
}

fn remove_other_versions(keep: &str) {
    let Ok(entries) = fs::read_dir(".") else { return };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with("ridx-") && name != keep {
            let _ = fs::remove_dir_all(entry.path());
        }
    }
}

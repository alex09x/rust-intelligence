/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! The index of an editing session: the published index of a workspace with the crates of the unsaved buffers
//! compiled again from the buffers' text. Nothing of the published index is written: the session's directory links
//! its shards, and each crate holding a buffer is compiled by `ridx-driver` (the direct path's arguments, recorded in
//! its sidecar) with `RIDX_BUFFERS` naming the buffers, into the session's directory. A crate with errors still gets
//! its shard (see the driver's `broken_crate`), and what the compiler reported on the buffers' text comes back with the
//! session (`SessionOutput::messages`).

use super::direct::args::sanitize_compiler_args_for_direct;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Builds in `session_dir` the index of `root` with `buffers` (source path, text) in place of their files, and returns
/// the directory to open it from. Fails when the workspace has no published index or no crate holds a buffer.
pub fn build_session_index(root: &Path, session_dir: &Path, buffers: &[(PathBuf, String)]) -> Result<SessionOutput, String> {
    let driver = super::driver::find_driver_binary().ok_or("ridx-driver not found")?;
    build_session_index_with(&driver, root, session_dir, buffers)
}

/// The variant a published manifest names, only as `variants/<id>` with a plain id: an absolute path or `..` would
/// make the session's copy the published directory itself, or put it outside the session (review of ca0c1ec7).
fn published_variant(manifest: &[u8]) -> Result<String, String> {
    let m: serde_json::Value = serde_json::from_slice(manifest).map_err(|e| format!("manifest: {e}"))?;
    let dir = m["variant_dir"].as_str().unwrap_or_default();
    let id = dir.strip_prefix("variants/").unwrap_or_default();
    let plain = !id.is_empty() && id != "." && id != ".." && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'));
    plain.then(|| dir.to_string()).ok_or_else(|| format!("manifest names an invalid variant directory {dir:?}"))
}

fn build_session_index_with(driver: &Path, root: &Path, session_dir: &Path, buffers: &[(PathBuf, String)]) -> Result<SessionOutput, String> {
    let base = root.join("target/ridx");
    // The published index is taken under its lock, released before compiling: a run of `ridx index` replaces shards
    // and sidecars in the variant before it publishes the manifest, so without the lock the copy could mix two of its
    // generations (review of 11310378). While a run holds it, no session is built (`BUSY`); the run's end reschedules.
    let lock = fs::OpenOptions::new().create(true).truncate(false).write(true).open(base.join(".index.lock")).map_err(|_| "no published index".to_string())?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(fs::TryLockError::WouldBlock) => return Err(BUSY.into()),
        // No lock, no copy (review of a21afbc0: a file system without locks copied without one).
        Err(fs::TryLockError::Error(e)) => return Err(format!("cannot lock {}: {e}", base.join(".index.lock").display())),
    }
    // One read of the manifest: the variant chosen and the manifest the session keeps are of the same publication.
    let manifest = fs::read(base.join("ridx_manifest.json")).map_err(|_| "no published index".to_string())?;
    let variant_rel = published_variant(&manifest)?;
    let published = base.join(&variant_rel);
    let variant = session_dir.join(&variant_rel);
    if session_dir.starts_with(&published) || published.starts_with(session_dir) {
        return Err(format!("session directory {} overlaps the published index", session_dir.display()));
    }
    let _ = fs::remove_dir_all(session_dir);
    fs::create_dir_all(variant.join(".deps")).map_err(|e| format!("session directory: {e}"))?;
    link_files(&published, &variant)?;
    link_files(&published.join(".deps"), &variant.join(".deps"))?;
    fs::write(session_dir.join("ridx_manifest.json"), &manifest).map_err(|e| format!("manifest: {e}"))?;
    drop(lock);

    // The buffers' text, and the list the driver reads.
    let texts = session_dir.join("buffers");
    fs::create_dir_all(&texts).map_err(|e| format!("buffers: {e}"))?;
    let mut list = serde_json::Map::new();
    let mut sources = HashSet::new();
    for (i, (src, text)) in buffers.iter().enumerate() {
        let file = texts.join(format!("{i}.rs"));
        fs::write(&file, text).map_err(|e| format!("buffer: {e}"))?;
        list.insert(src.display().to_string(), file.display().to_string().into());
        sources.insert(canonical(src));
    }
    let list_file = session_dir.join("buffers.json");
    fs::write(&list_file, serde_json::Value::Object(list).to_string()).map_err(|e| format!("buffer list: {e}"))?;

    let units = super::session_units::units_holding(&variant.join(".deps"), &sources, root);
    if units.is_empty() {
        return Err("no indexed crate holds the buffers".into());
    }
    let ctx = Ctx { driver, root, variant: &variant, list_file: &list_file, deps: session_dir.join("direct/deps"), incr: session_dir.join("direct/incremental") };
    fs::create_dir_all(&ctx.deps).and_then(|_| fs::create_dir_all(&ctx.incr)).map_err(|e| format!("direct directory: {e}"))?;

    // Compiled in waves, each unit once the edited libraries it uses are done, side by side within a wave (a file of
    // `ridx-format` is in its library, 1.7 s, and in its unit tests, 2.5 s). A unit reads the session's metadata of an
    // edited library that wrote it, else the saved one (review of ca0c1ec7: a buffer's new API used from another edited
    // crate was compiled against the saved library). Producer and consumer are matched by the metadata file itself
    // (`libname-<extra>`), whatever name the consumer binds it to (review of 11310378: a renamed dependency).
    let libs: HashSet<String> = units.iter().filter(|u| u.is_lib()).map(|u| u.artifact_stem()).collect();
    let mut fresh: HashSet<String> = HashSet::new();
    let mut settled: HashSet<String> = HashSet::new();
    let mut pending: Vec<&super::session_units::Unit> = units.iter().collect();
    let mut messages = Vec::new();
    while !pending.is_empty() {
        let ready = |u: &&super::session_units::Unit| u.extern_edges().iter().all(|(_, stem, _)| !libs.contains(stem) || *stem == u.artifact_stem() || settled.contains(stem));
        let (mut wave, rest): (Vec<_>, Vec<_>) = pending.into_iter().partition(ready);
        if wave.is_empty() {
            // A cycle cannot come from Cargo; compile the rest against what there is rather than not at all.
            wave = rest;
            pending = Vec::new();
        } else {
            pending = rest;
        }
        let running = wave
            .iter()
            .map(|u| {
                // Each bound file on its own, to a file of the session that replaces it, of its kind: a proc macro is
                // bound as a dylib the session (metadata only) never writes (review of a21afbc0), and one binding can
                // name an rlib and a dylib, of which only the first has a replacement (review of 503071cb).
                let args = super::session_units::rebind_externs(&u.args, |file| {
                    let stem = Path::new(file).file_stem()?.to_string_lossy().into_owned();
                    let name = Path::new(file).file_name()?.to_string_lossy().into_owned();
                    let replacement = ctx.deps.join(name.strip_suffix(".rlib").map_or(name.clone(), |s| format!("{s}.rmeta")));
                    (fresh.contains(&stem) && replacement.is_file()).then_some(replacement)
                });
                compile(&ctx, u, &args)
            })
            .collect::<Result<Vec<_>, _>>()?;
        for (unit, staged) in wave.iter().zip(running) {
            messages.extend(staged.finish(unit));
            if unit.is_lib() {
                let stem = unit.artifact_stem();
                if ctx.deps.join(format!("{stem}.rmeta")).is_file() {
                    fresh.insert(stem.clone());
                }
                settled.insert(stem);
            }
        }
    }
    Ok(SessionOutput { dir: session_dir.to_path_buf(), messages })
}

/// A session index built: the directory to open it from, and the compiler's messages (rustc's JSON diagnostics) on
/// the text it compiled, of every unit, in the order the units ran.
#[derive(Debug)]
pub struct SessionOutput {
    pub dir: PathBuf,
    pub messages: Vec<serde_json::Value>,
}

/// The error of a session that waits for a run of `ridx index` holding the index's lock.
pub const BUSY: &str = "the published index is being rebuilt";

/// `args` reporting in rustc's JSON, and only its diagnostics: the format and colour the unit was recorded with go.
fn json_diagnostics(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len() + 1);
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        if matches!(a.as_str(), "--error-format" | "--json" | "--color") {
            it.next();
        } else if !(a.starts_with("--error-format=") || a.starts_with("--json=") || a.starts_with("--color=")) {
            out.push(a);
        }
    }
    out.push("--error-format=json".into());
    out
}

/// The diagnostics among the lines rustc wrote to its standard error (`$message_type` "diagnostic"; a line of another
/// kind, or not JSON, is left out).
fn diagnostics_of(stderr: &str) -> Vec<serde_json::Value> {
    stderr
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .filter(|m| m.get("$message_type").is_none_or(|t| t == "diagnostic") && m.get("spans").is_some())
        .collect()
}

struct Ctx<'a> {
    driver: &'a Path,
    root: &'a Path,
    variant: &'a Path,
    list_file: &'a Path,
    deps: PathBuf,
    incr: PathBuf,
}

/// A unit's compile in flight, and the published shard it set aside.
struct Staged {
    child: std::process::Child,
    shard: Option<PathBuf>,
    backup: Option<PathBuf>,
    /// Reads the compiler's standard error as it comes: a unit with many errors would otherwise fill the pipe and wait
    /// for a reader while the session waits for it.
    stderr: Option<std::thread::JoinHandle<String>>,
}

impl Staged {
    /// Waits for the compile, and returns what the compiler reported. A unit the driver wrote no shard for (the
    /// compiler stopped before analysis, on a parse error) gets back the very shard set aside, not the published path
    /// read again, which a reindex may have replaced meanwhile (review of ca0c1ec7).
    fn finish(mut self, unit: &super::session_units::Unit) -> Vec<serde_json::Value> {
        let _ = self.child.wait();
        let messages = self.stderr.take().and_then(|h| h.join().ok()).map(|s| diagnostics_of(&s)).unwrap_or_default();
        match (&self.shard, &self.backup) {
            (Some(shard), Some(backup)) if !shard.is_file() => {
                let _ = fs::rename(backup, shard);
            }
            _ => {
                if let Some(backup) = &self.backup {
                    let _ = fs::remove_file(backup);
                }
                super::direct::units::refresh_aliases(&unit.sidecar, &unit.aliases);
            }
        }
        messages
    }
}

/// Starts the compile of `unit` with `args`, its recorded arguments with the externs the session replaces rebound.
fn compile(ctx: &Ctx, unit: &super::session_units::Unit, args: &[String]) -> Result<Staged, String> {
    // The link to the published shard is set aside first, so that the driver writes the session's own in its place
    // and never into the published one.
    let shard = unit.object.as_ref().and_then(|o| o.file_name()).map(|name| ctx.variant.join(name));
    let backup = shard.as_ref().map(|s| s.with_extension("ridx.published")).filter(|b| shard.as_ref().is_some_and(|s| fs::rename(s, b).is_ok()));
    let args = json_diagnostics(sanitize_compiler_args_for_direct(args, &ctx.deps, &ctx.incr, &HashSet::new()));
    let mut cmd = Command::new(ctx.driver);
    cmd.args(&args).current_dir(ctx.root).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    // The environment the unit was built with first, then the session's own over it: a recorded `RIDX_OUT_DIR` (a
    // build script can set any variable) must not send the compile into the published index (review of ca0c1ec7).
    cmd.envs(unit.env.iter().filter(|(k, _)| !k.starts_with("RIDX_")));
    cmd.env("RIDX_OUT_DIR", ctx.variant)
        .env("RIDX_CACHE_DIR", ridx_format::resolve_cache_dir())
        .env("RIDX_WORKSPACE_ROOT", ctx.root)
        .env("RIDX_BUFFERS", ctx.list_file)
        .env("RUSTC_BOOTSTRAP", "1");
    let target_kinds = ctx.variant.join("target_kinds.json");
    if target_kinds.is_file() {
        cmd.env("RIDX_TARGET_KINDS_FILE", target_kinds);
    } else {
        cmd.env_remove("RIDX_TARGET_KINDS_FILE");
    }
    if let Some(ld) = super::cargo::compute_sysroot_ld_path() {
        cmd.env("LD_LIBRARY_PATH", &ld).env("DYLD_FALLBACK_LIBRARY_PATH", &ld);
    }
    let mut child = cmd.spawn().map_err(|e| format!("ridx-driver: {e}"))?;
    let stderr = child.stderr.take().map(|mut pipe| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = std::io::Read::read_to_string(&mut pipe, &mut s);
            s
        })
    });
    Ok(Staged { child, shard, backup, stderr })
}

/// Puts every file of `from` into `to`, never its subdirectories: a shard as a link (they are large and only ever
/// replaced, never written in place), anything else as a copy, since the driver rewrites sidecars in place and a
/// link would carry that into the published index.
fn link_files(from: &Path, to: &Path) -> Result<(), String> {
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))?.flatten() {
        let path = entry.path();
        if path.is_file() {
            let dest = to.join(entry.file_name());
            let is_shard = path.extension().is_some_and(|x| x == "ridx");
            if !is_shard || fs::hard_link(&path, &dest).is_err() {
                fs::copy(&path, &dest).map_err(|e| format!("{}: {e}", path.display()))?;
            }
        }
    }
    Ok(())
}

pub(super) fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;

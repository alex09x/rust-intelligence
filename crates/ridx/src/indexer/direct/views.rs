/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! What the fast path remembers of a unit between runs, under `target/ridx-direct`:
//! - its interface (`interface/<unit>.json`), for the text its index and metadata were compiled from: the sources'
//!   digest it belongs to, the fingerprint, and the position of each interface token per file;
//! - its view (`views/<unit>.json`) of the units it depends on: the sources of each its shard's references point into,
//!   and when its metadata was compiled (a run counter), against the sidecar it describes. A unit compiled before one
//!   of its dependencies has metadata naming the dependency's older build, which rustc refuses next to the newer.
//!
//! Either record is only taken for the state it names: an interface for other sources, or a view of a sidecar
//! rewritten since (by Cargo), is none.

use super::interface::{file_interface, Fnv};
use super::units::{hash_matches, source_hashes};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

/// A unit's interface (see [`super::interface`]) for the sources whose digest is `sources`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UnitInterface {
    pub sources: u64,
    pub fingerprint: u64,
    /// Interface token positions, by the file's name as the sidecar records it; none for a file that is not Rust.
    pub files: BTreeMap<String, Vec<(u32, u32)>>,
}

/// A unit's view of its dependencies.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct UnitView {
    /// The sources digest of each dependency, as the unit's shard references it.
    pub deps: HashMap<String, u64>,
    /// When its metadata was compiled; 0 for metadata compiled by Cargo.
    pub built: u64,
}

/// The digest of a unit's own sources, as its sidecar records their hashes.
pub(crate) fn sources_digest(own_hashes: &HashMap<String, u64>) -> u64 {
    let mut files: Vec<(&String, &u64)> = own_hashes.iter().collect();
    files.sort();
    let mut h = Fnv::new();
    for (file, hash) in files {
        h.write(file.as_bytes());
        h.write(&[0]);
        h.write(&hash.to_le_bytes());
    }
    h.finish()
}

/// The interface of the sources the unit's sidecar records (`own_hashes`), read from disk; `None` when a file is no
/// longer that text, or a Rust file does not balance.
pub(crate) fn read_interface(work_dir: &Path, own_hashes: &HashMap<String, u64>) -> Option<UnitInterface> {
    let mut names: Vec<&String> = own_hashes.keys().collect();
    names.sort();
    let mut fingerprint = Fnv::new();
    let mut files = BTreeMap::new();
    for name in names {
        let bytes = fs::read(work_dir.join(name)).ok()?;
        if !hash_matches(source_hashes(&bytes), own_hashes[name]) {
            return None;
        }
        fingerprint.write(name.as_bytes());
        fingerprint.write(&[0]);
        if name.ends_with(".rs") {
            let file = file_interface(std::str::from_utf8(&bytes).ok()?)?;
            fingerprint.write(&file.hash.to_le_bytes());
            files.insert(name.clone(), file.positions);
        } else {
            fingerprint.write(&ridx_format::fnv1a_64(&bytes).to_le_bytes());
        }
    }
    Some(UnitInterface { sources: sources_digest(own_hashes), fingerprint: fingerprint.finish(), files })
}

/// Where the records of the fast path live, for a workspace.
#[derive(Debug, Clone)]
pub(crate) struct Records {
    dir: PathBuf,
}

impl Records {
    pub(crate) fn new(direct_dir: &Path) -> Self {
        Self { dir: direct_dir.to_path_buf() }
    }

    fn path(&self, kind: &str, unit: &str) -> PathBuf {
        self.dir.join(kind).join(format!("{unit}.json"))
    }

    fn interface_path(&self, unit: &str) -> PathBuf {
        self.dir.join("interface").join(format!("{unit}.bin"))
    }

    /// The interface recorded for `unit`, if it is of the sources `sources`; without its positions (`files` empty)
    /// unless `positions`, which only a moved dependency needs: a large crate's are megabytes.
    pub(crate) fn interface(&self, unit: &str, sources: u64, positions: bool) -> Option<UnitInterface> {
        let path = self.interface_path(unit);
        let bytes = if positions {
            fs::read(&path).ok()?
        } else {
            let mut head = vec![0; INTERFACE_MAGIC.len() + 16];
            std::io::Read::read_exact(&mut fs::File::open(&path).ok()?, &mut head).ok()?;
            head
        };
        let mut r = Bytes(bytes.strip_prefix(INTERFACE_MAGIC)?);
        if r.u64()? != sources {
            return None;
        }
        let fingerprint = r.u64()?;
        let mut files = BTreeMap::new();
        if !positions {
            return Some(UnitInterface { sources, fingerprint, files });
        }
        for _ in 0..r.u32()? {
            let len = r.u32()? as usize;
            let name = String::from_utf8(r.take(len)?.to_vec()).ok()?;
            let count = r.u32()? as usize;
            let positions = (0..count).map(|_| Some((r.u32()?, r.u32()?))).collect::<Option<Vec<_>>>()?;
            files.insert(name, positions);
        }
        Some(UnitInterface { sources, fingerprint, files })
    }

    /// Positions are many (one per interface token of the crate): written as little-endian words, not JSON.
    pub(crate) fn store_interface(&self, unit: &str, interface: &UnitInterface) {
        let mut out = INTERFACE_MAGIC.to_vec();
        out.extend(interface.sources.to_le_bytes());
        out.extend(interface.fingerprint.to_le_bytes());
        out.extend((interface.files.len() as u32).to_le_bytes());
        for (name, positions) in &interface.files {
            out.extend((name.len() as u32).to_le_bytes());
            out.extend(name.as_bytes());
            out.extend((positions.len() as u32).to_le_bytes());
            for &(line, col) in positions {
                out.extend(line.to_le_bytes());
                out.extend(col.to_le_bytes());
            }
        }
        write_atomically(&self.interface_path(unit), &out);
    }

    /// The view recorded for `unit`, if it describes the sidecar at `sidecar` as it is now.
    pub(crate) fn view(&self, unit: &str, sidecar: &Path) -> Option<UnitView> {
        let v: Value = serde_json::from_slice(&fs::read(self.path("views", unit)).ok()?).ok()?;
        if hex(&v["sidecar"])? != sidecar_digest(sidecar)? {
            return None;
        }
        let deps = v["deps"].as_object()?.iter().map(|(d, s)| Some((d.clone(), hex(s)?))).collect::<Option<_>>()?;
        Some(UnitView { deps, built: v["built"].as_u64()? })
    }

    pub(crate) fn store_view(&self, unit: &str, sidecar: &Path, view: &UnitView) {
        let Some(digest) = sidecar_digest(sidecar) else { return };
        let deps: serde_json::Map<String, Value> = view.deps.iter().map(|(d, s)| (d.clone(), json!(format!("{s:016x}")))).collect();
        let v = json!({ "sidecar": format!("{digest:016x}"), "deps": deps, "built": view.built });
        if let Ok(bytes) = serde_json::to_vec(&v) {
            write_atomically(&self.path("views", unit), &bytes);
        }
    }

    pub(crate) fn forget_view(&self, unit: &str) {
        let _ = fs::remove_file(self.path("views", unit));
    }
}

fn sidecar_digest(sidecar: &Path) -> Option<u64> {
    fs::read(sidecar).ok().map(|b| ridx_format::fnv1a_64(&b))
}

fn hex(v: &Value) -> Option<u64> {
    u64::from_str_radix(v.as_str()?, 16).ok()
}

const INTERFACE_MAGIC: &[u8] = b"RIDXIF1\n";

/// A little-endian reader over a record.
struct Bytes<'a>(&'a [u8]);

impl<'a> Bytes<'a> {
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let (head, rest) = (self.0.get(..n)?, self.0.get(n..)?);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        self.take(4).map(|b| u32::from_le_bytes(b.try_into().unwrap()))
    }

    fn u64(&mut self) -> Option<u64> {
        self.take(8).map(|b| u64::from_le_bytes(b.try_into().unwrap()))
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) {
    let Some(dir) = path.parent() else { return };
    if fs::create_dir_all(dir).is_err() {
        return;
    }
    let tmp = dir.join(format!(".{}.tmp.{}", path.file_name().unwrap_or_default().to_string_lossy(), std::process::id()));
    if fs::write(&tmp, bytes).is_ok() && fs::rename(&tmp, path).is_ok() {
        return;
    }
    let _ = fs::remove_file(&tmp);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_records_hold_only_for_the_state_they_name() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        fs::write(root.join("lib.rs"), "pub fn f() {}\n").unwrap();
        fs::write(root.join("data.txt"), "x").unwrap();
        let own: HashMap<String, u64> =
            [("lib.rs", "pub fn f() {}\n"), ("data.txt", "x")].iter().map(|(f, t)| (f.to_string(), ridx_format::normalized_source_hash(t.as_bytes()))).collect();
        let interface = read_interface(root, &own).expect("the recorded text is on disk");
        assert_eq!(interface.files["lib.rs"].len(), 6, "`pub fn f ( )` and the body: {:?}", interface.files);
        assert!(!interface.files.contains_key("data.txt"));

        let records = Records::new(&root.join("target/ridx-direct"));
        records.store_interface("a-1", &interface);
        assert_eq!(records.interface("a-1", interface.sources, true).as_ref(), Some(&interface));
        let head = records.interface("a-1", interface.sources, false).unwrap();
        assert_eq!((head.fingerprint, head.files.len()), (interface.fingerprint, 0), "the fingerprint alone");
        assert!(records.interface("a-1", interface.sources ^ 1, true).is_none(), "another text");

        let sidecar = root.join("a-1.json");
        fs::write(&sidecar, "{}").unwrap();
        let view = UnitView { deps: [("b-1".to_string(), 7)].into(), built: 3 };
        records.store_view("a-1", &sidecar, &view);
        assert_eq!(records.view("a-1", &sidecar), Some(view));
        fs::write(&sidecar, "{\"rewritten\":1}").unwrap();
        assert!(records.view("a-1", &sidecar).is_none(), "a sidecar rewritten since (Cargo)");

        fs::write(root.join("lib.rs"), "pub fn f() { 1; }\n").unwrap();
        assert!(read_interface(root, &own).is_none(), "the file moved on from the recorded text");
    }
}

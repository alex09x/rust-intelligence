/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which units of a fast-path run are compiled (prod-code, issue #7: every dependent of an edited crate was, though only
//! a body changed). A unit is compiled when its own sources changed, or when the interface of a crate it depends on did
//! (see [`super::interface`]). When only the positions of that crate's definitions moved, its shard's references are
//! moved to them instead ([`ridx_format::relocate`]), and it is not compiled.
//!
//! A unit not compiled keeps metadata built against its dependencies' older builds, which rustc refuses next to the
//! newer ones: before a unit is compiled, each unit it depends on whose metadata is older than one of its own
//! dependencies' is compiled first.

use super::views::{read_interface, sources_digest, Records, UnitInterface, UnitView};
use super::UnitOutcome;
use ridx_format::relocate::{relocate_reference_targets, Placement};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// A definition's position before an edit to its position after (line and column).
type PositionMap = HashMap<(u32, u32), (u32, u32)>;

/// A workspace unit: crate name, compiler arguments, environment, own source hashes, sidecar.
pub(crate) type UnitEntry = (String, Vec<String>, HashMap<String, String>, HashMap<String, u64>, PathBuf);

/// What a run knows of the units and their records, shared by the units compiled at once.
pub(crate) struct Cutoff<'a> {
    units: &'a HashMap<String, UnitEntry>,
    changed: &'a HashSet<String>,
    /// Every unit each unit depends on, directly or not, dependencies first.
    upstream: HashMap<String, Vec<String>>,
    /// The units another depends on: only theirs is an interface to compare.
    depended_on: HashSet<String>,
    records: Records,
    work_dir: &'a Path,
    /// The interface of each unit's index before this run.
    before: HashMap<String, UnitInterface>,
    /// And now: a unit compiled with errors, or whose text does not balance, has none.
    now: Mutex<HashMap<String, UnitInterface>>,
    built: Mutex<HashMap<String, u64>>,
    counter: AtomicU64,
    locks: HashMap<String, Mutex<()>>,
}

impl<'a> Cutoff<'a> {
    pub(crate) fn new(
        units: &'a HashMap<String, UnitEntry>,
        changed: &'a HashSet<String>,
        dependencies_of: &HashMap<String, HashSet<String>>,
        rdeps_of: &HashMap<String, HashSet<String>>,
        records: Records,
        work_dir: &'a Path,
    ) -> Self {
        let order = super::graph::topo_sort_crates(&units.keys().cloned().collect(), dependencies_of).unwrap_or_default();
        let rank: HashMap<&str, usize> = order.iter().enumerate().map(|(i, u)| (u.as_str(), i)).collect();
        let upstream = units.keys().map(|u| (u.clone(), transitive_dependencies(u, dependencies_of, &rank))).collect();
        let depended_on: HashSet<String> = units.keys().filter(|u| rdeps_of.get(*u).is_some_and(|r| !r.is_empty())).cloned().collect();
        let (mut before, mut built) = (HashMap::new(), HashMap::new());
        for (unit, (.., own, sidecar)) in units {
            built.insert(unit.clone(), records.view(unit, sidecar).map_or(0, |v| v.built));
            if !depended_on.contains(unit) {
                continue;
            }
            // Positions only of a unit compiled now: its dependents' references are moved from them.
            let recorded = records.interface(unit, sources_digest(own), changed.contains(unit)).or_else(|| {
                // Its index is of the text on disk: read now, the record a later edit is compared with.
                let read = (!changed.contains(unit)).then(|| read_interface(work_dir, own)).flatten()?;
                records.store_interface(unit, &read);
                Some(read)
            });
            before.extend(recorded.map(|i| (unit.clone(), i)));
        }
        let now = before.iter().filter(|(u, _)| !changed.contains(*u)).map(|(u, i)| (u.clone(), i.clone())).collect();
        let counter = AtomicU64::new(built.values().copied().max().unwrap_or(0) + 1);
        let locks = units.keys().map(|u| (u.clone(), Mutex::new(()))).collect();
        Self { units, changed, upstream, depended_on, records, work_dir, before, now: Mutex::new(now), built: Mutex::new(built), counter, locks }
    }

    /// Indexes a unit of the run, its dependencies done: compiles it, moves its references, or leaves it.
    pub(crate) fn visit(&self, unit: &str, compile: &(impl Fn(&str) -> UnitOutcome + Sync)) -> UnitOutcome {
        if self.changed.contains(unit) {
            return self.compile_fresh(unit, compile);
        }
        let Some((.., sidecar)) = self.units.get(unit) else { return UnitOutcome::Failed };
        let Some(view) = self.records.view(unit, sidecar) else { return self.compile_fresh(unit, compile) };
        let Some(moved) = self.moved_dependencies(unit, &view) else { return self.compile_fresh(unit, compile) };
        if moved.is_empty() {
            return UnitOutcome::Skipped;
        }
        match self.relocate(unit, &moved) {
            Some(count) => {
                let mut view = view;
                view.deps.extend(moved.iter().map(|(dep, _, current)| (dep.clone(), current.sources)));
                self.records.store_view(unit, sidecar, &view);
                eprintln!("[ridx-direct] {unit} not compiled: what it depends on kept its interface, {count} references moved");
                UnitOutcome::Skipped
            }
            None => self.compile_fresh(unit, compile),
        }
    }

    /// The dependencies whose definitions moved since `unit`'s shard was written, with their interface then and now;
    /// `None` when one's interface changed, or is not known then or now.
    fn moved_dependencies(&self, unit: &str, view: &UnitView) -> Option<Vec<(String, UnitInterface, UnitInterface)>> {
        let now = self.now.lock().unwrap();
        let mut moved = Vec::new();
        for dep in &self.upstream[unit] {
            let (current, &seen) = (now.get(dep)?, view.deps.get(dep)?);
            if seen == current.sources {
                continue;
            }
            let old = self.before.get(dep).filter(|old| old.sources == seen && old.fingerprint == current.fingerprint)?;
            moved.push((dep.clone(), old.clone(), current.clone()));
        }
        Some(moved)
    }

    /// Compiles `unit` after each unit it depends on whose metadata is older than a dependency's. One that compiles
    /// with errors has no newer metadata to give: the unit is compiled as before, against its last.
    fn compile_fresh(&self, unit: &str, compile: &(impl Fn(&str) -> UnitOutcome + Sync)) -> UnitOutcome {
        for dep in &self.upstream[unit] {
            let _held = self.locks[dep].lock().unwrap();
            if self.is_stale(dep) {
                eprintln!("[ridx-direct] {dep} compiled again: its metadata is older than a dependency's");
                if self.compile_and_record(dep, compile) == UnitOutcome::Failed {
                    return UnitOutcome::Failed;
                }
            }
        }
        let _held = self.locks[unit].lock().unwrap();
        self.compile_and_record(unit, compile)
    }

    fn is_stale(&self, unit: &str) -> bool {
        let built = self.built.lock().unwrap();
        let own = built.get(unit).copied().unwrap_or(0);
        self.upstream[unit].iter().any(|d| built.get(d).copied().unwrap_or(0) > own)
    }

    fn compile_and_record(&self, unit: &str, compile: &(impl Fn(&str) -> UnitOutcome + Sync)) -> UnitOutcome {
        let outcome = compile(unit);
        let Some((.., sidecar)) = self.units.get(unit) else { return outcome };
        if outcome == UnitOutcome::Failed {
            self.records.forget_view(unit);
            return outcome;
        }
        self.built.lock().unwrap().insert(unit.to_string(), self.counter.fetch_add(1, Ordering::SeqCst));
        // The sources the driver compiled, as it recorded them. A unit with errors gives its dependents no interface
        // (they are left out, and compiled once it has one).
        let read = if self.depended_on.contains(unit) && outcome == UnitOutcome::Built {
            super::units::recorded_own_hashes(sidecar, self.work_dir).and_then(|own| read_interface(self.work_dir, &own))
        } else {
            None
        };
        let mut now = self.now.lock().unwrap();
        match read {
            Some(interface) => {
                self.records.store_interface(unit, &interface);
                now.insert(unit.to_string(), interface);
            }
            None => {
                now.remove(unit);
            }
        }
        let deps = self.upstream[unit].iter().filter_map(|d| now.get(d).map(|i| (d.clone(), i.sources))).collect();
        drop(now);
        let built = self.built.lock().unwrap()[unit];
        self.records.store_view(unit, sidecar, &UnitView { deps, built });
        outcome
    }

    /// Moves the references of `unit`'s shard into the files of `moved` (a dependency, its interface before and now);
    /// `None` when one cannot be placed, and the unit is to be compiled.
    fn relocate(&self, unit: &str, moved: &[(String, UnitInterface, UnitInterface)]) -> Option<usize> {
        let (.., env, _, sidecar) = self.units.get(unit)?;
        let side: serde_json::Value = serde_json::from_slice(&std::fs::read(sidecar).ok()?).ok()?;
        let shard = PathBuf::from(side["object_path"].as_str()?);
        let manifest_dir = env.get("CARGO_MANIFEST_DIR").map(PathBuf::from);
        // Each moved file, by its canonical path: old position to new.
        let mut files: HashMap<PathBuf, PositionMap> = HashMap::new();
        for (_, old, current) in moved {
            for (name, positions) in &current.files {
                let before = old.files.get(name)?;
                if before == positions {
                    continue;
                }
                let path = self.work_dir.join(name).canonicalize().ok()?;
                files.insert(path, before.iter().copied().zip(positions.iter().copied()).collect());
            }
        }
        let mut resolved: HashMap<String, Vec<PathBuf>> = HashMap::new();
        let moved_count = relocate_reference_targets(&shard, |target_file, line, col| {
            let candidates = resolved.entry(target_file.to_string()).or_insert_with(|| candidate_files(target_file, self.work_dir, manifest_dir.as_deref()));
            let hits: Vec<&PathBuf> = candidates.iter().filter(|c| files.contains_key(*c)).collect();
            match hits.as_slice() {
                // A name no file stands for (rewritten by `--remap-path-prefix`, review of 2a030462) may be a moved
                // file's when it ends as one does.
                [] if candidates.is_empty() && Path::new(target_file).file_name().is_some_and(|n| files.keys().any(|f| f.file_name() == Some(n))) => Placement::Unknown,
                [] => Placement::Same,
                [file] if candidates.len() == 1 => files[*file].get(&(line, col)).map_or(Placement::Unknown, |&(l, c)| Placement::Moved(l, c)),
                // Two files the name may stand for, one of them moved: not told apart.
                _ => Placement::Unknown,
            }
        });
        moved_count.ok().flatten()
    }
}

/// After a complete Cargo index each unit was compiled against its dependencies as they are now: records the interface
/// of each unit another depends on and, for each unit without one, a view of them (its metadata Cargo's), so the first
/// edit after it is compared with them too.
pub(crate) fn record_after_cargo(variant_out_dir: &Path, work_dir: &Path) {
    let started = std::time::Instant::now();
    let Some((_, units, _)) = super::units::read_units(&variant_out_dir.join(".deps"), work_dir) else { return };
    let count = units.len();
    let (dependencies_of, rdeps_of) = super::graph::build_workspace_dependency_graphs(&units);
    let units: HashMap<String, UnitEntry> = units.into_iter().map(|(t, n, a, e, f, p)| (t, (n, a, e, f, p))).collect();
    let order = super::graph::topo_sort_crates(&units.keys().cloned().collect(), &dependencies_of).unwrap_or_default();
    let rank: HashMap<&str, usize> = order.iter().enumerate().map(|(i, u)| (u.as_str(), i)).collect();
    let records = Records::new(&work_dir.join("target/ridx-direct"));
    let sources: HashMap<&str, u64> = units.iter().map(|(u, (.., own, _))| (u.as_str(), sources_digest(own))).collect();
    for (unit, (.., own, sidecar)) in &units {
        if rdeps_of.get(unit).is_some_and(|r| !r.is_empty()) && records.interface(unit, sources[unit.as_str()], false).is_none() {
            if let Some(interface) = read_interface(work_dir, own) {
                records.store_interface(unit, &interface);
            }
        }
        if records.view(unit, sidecar).is_none() {
            let deps = transitive_dependencies(unit, &dependencies_of, &rank).into_iter().filter_map(|d| sources.get(d.as_str()).map(|&s| (d, s))).collect();
            records.store_view(unit, sidecar, &UnitView { deps, built: 0 });
        }
    }
    eprintln!("[ridx-direct] interfaces and views of {count} units recorded in {:.0} ms", started.elapsed().as_secs_f64() * 1000.0);
}

/// The existing files a reference's target file may name: as given when absolute, else relative to the workspace (as
/// rustc names a crate's files compiled from there) or to the referring package (as the driver records its own).
fn candidate_files(name: &str, work_dir: &Path, manifest_dir: Option<&Path>) -> Vec<PathBuf> {
    let path = Path::new(name);
    let tried: Vec<PathBuf> = if path.is_absolute() { vec![path.to_path_buf()] } else { [Some(work_dir), manifest_dir].into_iter().flatten().map(|d| d.join(path)).collect() };
    let mut out: Vec<PathBuf> = tried.into_iter().filter_map(|p| p.canonicalize().ok()).collect();
    out.dedup();
    out
}

/// The units `unit` depends on, directly or not, in `rank` order (dependencies first).
fn transitive_dependencies(unit: &str, dependencies_of: &HashMap<String, HashSet<String>>, rank: &HashMap<&str, usize>) -> Vec<String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut stack: Vec<&str> = dependencies_of.get(unit).into_iter().flatten().map(String::as_str).collect();
    while let Some(d) = stack.pop() {
        if d != unit && seen.insert(d) {
            stack.extend(dependencies_of.get(d).into_iter().flatten().map(String::as_str));
        }
    }
    let mut out: Vec<String> = seen.into_iter().map(str::to_string).collect();
    out.sort_by_key(|d| rank.get(d.as_str()).copied().unwrap_or(usize::MAX));
    out
}

#[cfg(test)]
#[path = "cutoff_tests.rs"]
mod tests;

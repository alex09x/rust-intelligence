/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

use super::*;
use std::fs;
use std::sync::Mutex as StdMutex;

/// A workspace of three units: `b` uses `a`, `c` uses both. Each sidecar records its own file; `b`'s shard references
/// `a::f`.
struct Ws {
    root: PathBuf,
    units: HashMap<String, UnitEntry>,
    deps: HashMap<String, HashSet<String>>,
    rdeps: HashMap<String, HashSet<String>>,
}

impl Ws {
    fn new(dir: &Path) -> Self {
        let root = dir.canonicalize().unwrap();
        let mut ws = Self { root, units: HashMap::new(), deps: HashMap::new(), rdeps: HashMap::new() };
        for (unit, deps) in [("a", vec![]), ("b", vec!["a"]), ("c", vec!["a", "b"])] {
            fs::create_dir_all(ws.root.join(unit)).unwrap();
            ws.deps.insert(unit.into(), deps.iter().map(|d| d.to_string()).collect());
            for d in deps {
                ws.rdeps.entry(d.into()).or_default().insert(unit.into());
            }
        }
        ws.edit("a", "pub fn f() -> u32 {\n    1\n}\n");
        ws.edit("b", "pub fn g() -> u32 {\n    a::f()\n}\n");
        ws.edit("c", "pub fn h() -> u32 {\n    b::g()\n}\n");
        for unit in ["a", "b", "c"] {
            ws.compiled(unit);
        }
        let mut shard = ridx_format::IndexWriter::new("b", "0.1.0");
        shard.add_reference(ridx_format::RawReference {
            file: "b/lib.rs".into(),
            start_line: 2,
            start_col: 8,
            end_line: 2,
            end_col: 9,
            target_def_id: None,
            target_path: "a::f".into(),
            target_file: Some("a/lib.rs".into()),
            target_unit: Some("unit-a".into()),
            target_line: 1,
            target_col: 8,
            kind: ridx_format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
        shard.write_to(&mut fs::File::create(ws.root.join("b.ridx")).unwrap()).unwrap();
        for unit in ["a", "c"] {
            ridx_format::IndexWriter::new(unit, "0.1.0").write_to(&mut fs::File::create(ws.root.join(format!("{unit}.ridx"))).unwrap()).unwrap();
        }
        ws
    }

    fn edit(&self, unit: &str, text: &str) {
        fs::write(self.root.join(unit).join("lib.rs"), text).unwrap();
    }

    /// What the driver leaves after compiling `unit`: its sidecar with the hashes of the text it read.
    fn compiled(&mut self, unit: &str) {
        let file = format!("{unit}/lib.rs");
        let hash = ridx_format::normalized_source_hash(&fs::read(self.root.join(&file)).unwrap());
        let sidecar = self.root.join(format!("{unit}.json"));
        let object = self.root.join(format!("{unit}.ridx")).display().to_string();
        fs::write(&sidecar, serde_json::to_vec(&serde_json::json!({ "file_hashes": { file.clone(): hash }, "object_path": object })).unwrap()).unwrap();
        self.units.insert(unit.into(), (unit.into(), vec![], HashMap::new(), HashMap::from([(file, hash)]), sidecar));
    }

    /// One run with `changed` edited: the units compiled, in order, and each unit's outcome.
    fn run(&mut self, changed: &[&str]) -> (Vec<String>, HashMap<String, UnitOutcome>) {
        let changed: HashSet<String> = changed.iter().map(|u| u.to_string()).collect();
        let records = Records::new(&self.root.join("target/ridx-direct"));
        let log = StdMutex::new(Vec::new());
        let compile = |unit: &str| {
            log.lock().unwrap().push(unit.to_string());
            // The sidecar the driver writes, with this text's hashes.
            let file = format!("{unit}/lib.rs");
            let hash = ridx_format::normalized_source_hash(&fs::read(self.root.join(&file)).unwrap());
            let object = self.root.join(format!("{unit}.ridx")).display().to_string();
            fs::write(self.root.join(format!("{unit}.json")), serde_json::to_vec(&serde_json::json!({ "file_hashes": { file: hash }, "object_path": object })).unwrap()).unwrap();
            UnitOutcome::Built
        };
        let outcomes = {
            let cutoff = Cutoff::new(&self.units, &changed, &self.deps, &self.rdeps, records, &self.root);
            ["a", "b", "c"].iter().map(|u| (u.to_string(), cutoff.visit(u, &compile))).collect()
        };
        for unit in log.lock().unwrap().clone() {
            self.compiled(&unit);
        }
        (log.into_inner().unwrap(), outcomes)
    }

    fn b_target(&self) -> (u32, u32) {
        let reader = ridx_format::IndexReader::open(self.root.join("b.ridx")).unwrap();
        let r = reader.references()[0];
        (r.target_line(), r.target_col())
    }
}

#[test]
fn test_dependents_of_an_edit_that_keeps_the_interface_are_not_compiled() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Ws::new(dir.path());
    // The first run after a Cargo index knows no views: what depends on the edit is compiled, and recorded.
    let (compiled, _) = ws.run(&["a"]);
    assert_eq!(compiled, ["a", "b", "c"]);

    // A body and a line above `f`: `b` and `c` are left, `b`'s reference follows `f` to its new line.
    ws.edit("a", "// moved\npub fn f() -> u32 {\n    let one = 1;\n    one\n}\n");
    let (compiled, outcomes) = ws.run(&["a"]);
    assert_eq!(compiled, ["a"]);
    assert_eq!(outcomes["b"], UnitOutcome::Skipped);
    assert_eq!(outcomes["c"], UnitOutcome::Skipped);
    assert_eq!(ws.b_target(), (2, 8), "the reference moved with the definition");

    // An edit of `c` compiles it against `b` built anew: `b`'s metadata names the `a` before.
    ws.edit("c", "pub fn h() -> u32 {\n    b::g() + 1\n}\n");
    let (compiled, _) = ws.run(&["c"]);
    assert_eq!(compiled, ["b", "c"], "the dependency whose metadata is older than its own dependency's first");

    // A changed signature reaches the dependents.
    ws.edit("a", "// moved\npub fn f() -> u64 {\n    1\n}\n");
    let (compiled, _) = ws.run(&["a"]);
    assert_eq!(compiled, ["a", "b", "c"]);
}

/// Review of 2a030462: a target file named as no file on disk (`--remap-path-prefix`) was taken for one not moved.
#[test]
fn test_a_target_file_no_path_resolves_compiles_the_dependent() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Ws::new(dir.path());
    let mut shard = ridx_format::IndexWriter::new("b", "0.1.0");
    for (file, line) in [("/virtual-source/lib.rs", 1), ("/rustc/0000/library/core/src/option.rs", 572)] {
        shard.add_reference(ridx_format::RawReference {
            file: "b/lib.rs".into(),
            start_line: 2,
            start_col: 8,
            end_line: 2,
            end_col: 9,
            target_def_id: None,
            target_path: "a::f".into(),
            target_file: Some(file.into()),
            target_unit: None,
            target_line: line,
            target_col: 8,
            kind: ridx_format::DefKind::Fn,
            is_write: false,
            is_macro_expansion: false,
        });
    }
    shard.write_to(&mut fs::File::create(ws.root.join("b.ridx")).unwrap()).unwrap();
    ws.run(&["a"]);
    ws.edit("a", "\npub fn f() -> u32 {\n    1\n}\n");
    assert_eq!(ws.run(&["a"]).0, ["a", "b"], "`lib.rs` may be `a`'s moved file; `option.rs` is no file of `a`");
}

#[test]
fn test_a_reference_that_cannot_be_placed_compiles_the_dependent() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Ws::new(dir.path());
    // `b`'s reference names a position no interface token of `a` starts at. Its view is recorded after (review of
    // 2a030462: recorded before, the shard written since made it no view, and `b` was compiled for that).
    let mut shard = ridx_format::IndexWriter::new("b", "0.1.0");
    shard.add_reference(ridx_format::RawReference {
        file: "b/lib.rs".into(),
        start_line: 2,
        start_col: 8,
        end_line: 2,
        end_col: 9,
        target_def_id: None,
        target_path: "a::f".into(),
        target_file: Some("a/lib.rs".into()),
        target_unit: None,
        target_line: 2,
        target_col: 5,
        kind: ridx_format::DefKind::Fn,
        is_write: false,
        is_macro_expansion: false,
    });
    shard.write_to(&mut fs::File::create(ws.root.join("b.ridx")).unwrap()).unwrap();
    ws.run(&["a"]);
    ws.edit("a", "\npub fn f() -> u32 {\n    1\n}\n");
    let (compiled, outcomes) = ws.run(&["a"]);
    assert_eq!(compiled, ["a", "b"], "`b` compiled");
    assert_eq!(outcomes["c"], UnitOutcome::Skipped, "`c` sees the same interfaces: left, its metadata now older than `b`'s");
    ws.edit("c", "pub fn h() -> u32 {\n    b::g() + 2\n}\n");
    assert_eq!(ws.run(&["c"]).0, ["c"], "`b` is no older than its dependency: `c` alone");
}

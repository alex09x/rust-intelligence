/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! `ridx/safeDelete`: what it deletes, what it refuses, and the uses that keep an item.

use super::*;
use crate::lsp::editing::tests::code_action_assists_tests::assert_rustc_compiles;
use ridx_format::{DefKind, IndexWriter, RawDefinition, RawReference};

/// `doc` with the item named at `$0` deleted, or why not.
fn deleted(doc: &str) -> Result<String, &'static str> {
    let offset = doc.find("$0").unwrap();
    let doc = doc.replacen("$0", "", 1);
    let span = plan(&doc, offset)?.span;
    let mut out = doc.clone();
    out.replace_range(span, "");
    Ok(out)
}

#[test]
fn test_an_item_goes_with_its_doc_and_its_lines() {
    let got = deleted("pub fn a() {}\n\n/// Twice `x`.\n#[inline]\npub fn $0twice(x: u32) -> u32 {\n    x * 2\n}\n\npub fn b() {}\n").unwrap();
    assert_eq!(got, "pub fn a() {}\n\npub fn b() {}\n");
    assert_rustc_compiles(&got, "safe delete");
    let got = deleted("pub struct S;\nimpl S {\n    pub fn $0one() -> u8 {\n        1\n    }\n    pub fn two() -> u8 {\n        2\n    }\n}\n").unwrap();
    assert_eq!(got, "pub struct S;\nimpl S {\n    pub fn two() -> u8 {\n        2\n    }\n}\n", "a method of an inherent impl");
    assert_eq!(deleted("pub const $0A: u8 = 1; pub const B: u8 = 2;\n").unwrap(), "pub const B: u8 = 2;\n", "not alone on its line");
    assert_eq!(deleted("mod $0inner {\n    pub fn f() {}\n}\n").unwrap(), "");
    // Review of d753d993: ordinary comments that look like doc comments stay.
    assert_eq!(deleted("//// separator\n/*** block */\npub fn $0f() {}\n").unwrap(), "//// separator\n/*** block */\n");
}

#[test]
fn test_what_its_own_deletion_would_break_is_refused() {
    assert_eq!(deleted("pub trait T {\n    fn $0f(&self);\n}\n"), Err("traitItem"));
    assert_eq!(deleted("pub trait T {\n    fn f(&self);\n}\npub struct S;\nimpl T for S {\n    fn $0f(&self) {}\n}\n"), Err("traitItem"));
    assert_eq!(deleted("mod $0other;\n"), Err("moduleFile"), "its file would stay");
    assert_eq!(deleted("pub fn f() {\n    let $0x = 1;\n}\n"), Err("notAnItem"));
    // Review of d753d993: used though nothing in the source names it.
    assert_eq!(deleted("fn $0main() {}\n"), Err("entryPoint"));
    assert_eq!(deleted("#[no_mangle]\npub extern \"C\" fn $0hook() {}\n"), Err("externallyUsed"));
    // Review of f4f8b604: raw spellings, a member used from outside, an impl, a registration the loader runs.
    assert_eq!(deleted("fn $0r#main() {}\n"), Err("entryPoint"));
    assert_eq!(deleted("#[r#no_mangle]\npub extern \"C\" fn $0hook() {}\n"), Err("externallyUsed"));
    assert_eq!(deleted("mod $0m {\n    #[no_mangle]\n    pub extern \"C\" fn api() {}\n}\n"), Err("externallyUsed"));
    assert_eq!(deleted("pub trait Marker {}\npub struct S;\nmod $0m {\n    impl super::Marker for super::S {}\n}\n"), Err("containsImpl"));
    assert_eq!(deleted("#[used]\n#[link_section = \".init_array\"]\nstatic $0INIT: extern \"C\" fn() = init;\nextern \"C\" fn init() {}\n"), Err("externallyUsed"));
    // Review of 4009c2c4: the allocator the compiler selects.
    assert_eq!(deleted("use std::alloc::System;\n#[global_allocator]\nstatic $0A: System = System;\n"), Err("externallyUsed"));
}

/// Review of 4009c2c4: a use the compiler recorded in a macro's expansion keeps the item; a use's columns on a first
/// line after a byte order mark are the client's; a workspace's own `.rustup` directory is no toolchain's.
#[test]
fn test_every_stored_edge_and_its_client_place() {
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let mut conv = PositionConverter::new();
    let (_d, path, index) = workspace_with(lib, "// a caller\n\npub fn caller() {}\n", &[("used", DefKind::Fn, 1, 8)], &[(3, 5, 0)], true);
    assert_eq!(handle_safe_delete(&at(&path, 0, 8), Some(&index), &OverlayManager::new(), &mut conv)["reason"], "used", "a macro's expansion");
    let caller = "\u{feff}pub fn caller() { crate::used() }\n";
    let (_d, path, index) = workspace_with(lib, caller, &[("used", DefKind::Fn, 1, 8)], &[(1, 27, 0)], false);
    let got = handle_safe_delete(&at(&path, 0, 8), Some(&index), &OverlayManager::new(), &mut PositionConverter::new());
    assert_eq!(got["remainingUses"][0]["range"]["start"]["character"], 27, "{got}");
    assert!(!crate::lsp::editing::safe_delete_fresh::cached(std::path::Path::new("/w/.rustup/caller.rs")));
}

/// An inline module's declarations are all checked.
#[test]
fn test_a_container_names_its_members() {
    let doc = "mod m {\n    pub fn child() {}\n    pub struct S;\n}\n";
    let names: Vec<&str> = plan(doc, 4).unwrap().names.iter().map(|r| &doc[r.clone()]).collect();
    assert_eq!(names, ["m", "child", "S"]);
}

/// A workspace whose `src/lib.rs` is `lib` with `defs` (name, kind, 1-based line and column), indexed from that very
/// text, and whose `src/other.rs` holds `refs` (1-based line and column, the index of the definition used).
fn workspace(lib: &str, defs: &[(&str, DefKind, u32, u32)], refs: &[(u32, u32, usize)]) -> (tempfile::TempDir, std::path::PathBuf, WorkspaceIndex) {
    workspace_with(lib, "// a caller\n\npub fn caller() {}\n", defs, refs, false)
}

/// [`workspace`] with `caller` as `src/other.rs`, its references recorded as a macro's expansion when `in_macro`.
fn workspace_with(
    lib: &str,
    caller: &str,
    defs: &[(&str, DefKind, u32, u32)],
    refs: &[(u32, u32, usize)],
    in_macro: bool,
) -> (tempfile::TempDir, std::path::PathBuf, WorkspaceIndex) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    let (lib_path, other) = (root.join("src/lib.rs"), root.join("src/other.rs"));
    std::fs::write(&lib_path, lib).unwrap();
    std::fs::write(&other, caller).unwrap();
    let mut w = IndexWriter::new("w", "0.1.0");
    w.add_file_hash(&lib_path.to_string_lossy(), ridx_format::normalized_source_hash(lib.as_bytes()));
    w.add_file_hash(&other.to_string_lossy(), ridx_format::normalized_source_hash(caller.as_bytes()));
    w.get_or_insert_file(&other.to_string_lossy());
    let paths: Vec<String> =
        defs.iter().enumerate().map(|(i, d)| defs.iter().take(i).filter(|m| m.1 == DefKind::Mod).map(|m| format!("{}::", m.0)).collect::<String>() + d.0).collect();
    for (i, (name, kind, line, col)) in defs.iter().enumerate() {
        let path = paths[i].clone();
        w.add_definition(RawDefinition {
            def_id: i as u32 + 1,
            kind: *kind,
            visibility: 0,
            is_macro_expansion: false,
            file: lib_path.to_string_lossy().to_string(),
            name: name.to_string(),
            start_line: *line,
            start_col: *col,
            end_line: *line,
            end_col: col + name.chars().count() as u32,
            path,
            signature: None,
        });
    }
    for (line, col, def) in refs {
        let (name, kind, tl, tc) = defs[*def];
        w.add_reference(RawReference {
            file: other.to_string_lossy().to_string(),
            start_line: *line,
            start_col: *col,
            end_line: *line,
            end_col: col + name.chars().count() as u32,
            target_def_id: Some(*def as u32 + 1),
            target_path: paths[*def].clone(),
            target_file: Some(lib_path.to_string_lossy().to_string()),
            target_unit: None,
            target_line: tl,
            target_col: tc,
            kind,
            is_write: false,
            is_macro_expansion: in_macro,
        });
    }
    let shard = root.join("w.ridx");
    w.write_to(&mut std::fs::File::create(&shard).unwrap()).unwrap();
    let index = WorkspaceIndex::open(&shard).unwrap();
    (dir, lib_path, index)
}

fn at(lib: &std::path::Path, line: u32, character: u32) -> Value {
    json!({ "params": { "textDocument": { "uri": format!("file://{}", lib.display()) }, "position": { "line": line, "character": character } } })
}

/// prod-code, m=20226: an item used is refused with its uses, an unused one gets its edit.
#[test]
fn test_the_uses_keep_an_item() {
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)], &[(3, 5, 0)]);
    let (ov, mut conv) = (OverlayManager::new(), PositionConverter::new());
    let used = handle_safe_delete(&at(&path, 0, 8), Some(&index), &ov, &mut conv);
    assert_eq!((used["reason"].as_str(), used["remainingUses"][0]["range"]["start"]["line"].as_u64()), (Some("used"), Some(2)), "{used}");
    let unused = handle_safe_delete(&at(&path, 1, 8), Some(&index), &ov, &mut conv);
    let uri = format!("file://{}", path.display());
    assert_eq!(unused["edit"]["changes"][&uri][0]["range"], json!({ "start": { "line": 1, "character": 0 }, "end": { "line": 2, "character": 0 } }), "{unused}");
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), None, &ov, &mut conv)["reason"], "unknownSymbol", "no index: its uses are not known");
    // prod-code, m=20268: a file saved since the index was built may hold a use of it.
    std::fs::write(path.with_file_name("added.rs"), "pub fn added() { crate::unused() }\n").unwrap();
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &ov, &mut conv)["reason"], "staleIndex");
}

/// Review of 9effe92d: the index is current by the sources' content, whatever the server was told of, by whichever
/// path the index was opened or published: a file it did not read that may use the item makes it stale, one that
/// cannot does not.
#[test]
fn test_an_index_is_current_by_the_content_of_the_sources() {
    let lib = "pub fn used() {}\npub fn unused() {}\nmacro_rules! call_it {\n    () => {\n        $crate::unused()\n    };\n}\nmacro_rules! outer {\n    () => {\n        call_it!()\n    };\n}\n";
    let defs = [("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)];
    let mut conv = PositionConverter::new();
    let reason = |files: &[(&str, &str)], ov: &OverlayManager, conv: &mut PositionConverter| {
        let (_d, path, index) = workspace(lib, &defs, &[]);
        let src = path.parent().unwrap().to_path_buf();
        for (name, text) in files {
            let f = src.join(name);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let got = handle_safe_delete(&at(&path, 1, 8), Some(&index), ov, conv);
        got["reason"].as_str().map_or("deleted".to_string(), str::to_string)
    };
    let ov = OverlayManager::new();
    assert_eq!(reason(&[], &ov, &mut conv), "deleted");
    assert_eq!(reason(&[("plain.rs", "pub fn plain() -> u8 { 1 }\n")], &ov, &mut conv), "deleted", "a new file naming nothing of it");
    // A directory named `target` is a source directory; Cargo's build directory (a `CACHEDIR.TAG`) is not.
    assert_eq!(reason(&[("target/caller.rs", "pub fn c() { crate::unused() }\n")], &ov, &mut conv), "staleIndex");
    assert_eq!(reason(&[("out/CACHEDIR.TAG", "Signature: 8a477f597d28d172789f06886806bc55\n"), ("out/gen.rs", "fn g() { unused() }\n")], &ov, &mut conv), "deleted");
    // A raw spelling, a name beyond ASCII (which may be it in another normal form), a macro whose body names it, also
    // through another macro.
    assert_eq!(reason(&[("raw.rs", "pub fn c() { crate::r#unused() }\n")], &ov, &mut conv), "staleIndex");
    assert_eq!(reason(&[("kelvin.rs", "pub fn \u{212a}() {}\n")], &ov, &mut conv), "staleIndex");
    assert_eq!(reason(&[("mac.rs", "pub fn c() { call_it!(); }\n")], &ov, &mut conv), "staleIndex");
    assert_eq!(reason(&[("mac2.rs", "pub fn c() { outer!(); }\n")], &ov, &mut conv), "staleIndex");
    assert_eq!(reason(&[("mac3.rs", "pub fn c() { println!(\"x\"); }\n")], &ov, &mut conv), "deleted", "a macro naming nothing of it");
    // The Cargo workspace as Cargo resolves it: a package of its own nested in a repository is read alone, and a
    // workspace of its own nested in it is not read.
    let (_d, path, index) = workspace(lib, &defs, &[]);
    let root = path.parent().unwrap().parent().unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n").unwrap();
    std::fs::create_dir_all(root.join("fixture/src")).unwrap();
    std::fs::write(root.join("fixture/Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\n\n[workspace]\n").unwrap();
    std::fs::write(root.join("fixture/src/lib.rs"), "pub fn unused() {}\n").unwrap();
    // Nor is the build directory Cargo names, where ridx keeps copies of the texts it indexed.
    std::fs::create_dir_all(root.join("target/ridx/sources")).unwrap();
    std::fs::write(root.join("target/ridx/sources/0.rs"), "pub fn c() { crate::unused() }\n").unwrap();
    assert!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), &mut conv)["edit"].is_object());
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = [\"fixture\"]\n").unwrap();
    std::fs::write(root.join("fixture/Cargo.toml"), "[package]\nname = \"fx\"\nversion = \"0.1.0\"\n").unwrap();
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), &mut conv)["reason"], "staleIndex", "a member is read");
    // A buffer of a file not saved yet.
    let (_d, path, index) = workspace(lib, &defs, &[]);
    let new = path.with_file_name("new.rs");
    let mut ov = OverlayManager::new();
    ov.open_document(&format!("file://{}", new.display()), &new.to_string_lossy(), 1, "pub fn c() { crate::unused() }\n".to_string());
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &ov, &mut conv)["reason"], "staleIndex");
}

/// Review of d753d993: the index must be of this text and know this item; a member's use keeps its container; an
/// unsaved caller elsewhere keeps the item; a name with a combining mark is no panic.
#[test]
fn test_safe_delete_fails_closed() {
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)], &[(3, 5, 0)]);
    let mut conv = PositionConverter::new();
    let mut ov = OverlayManager::new();
    let uri = format!("file://{}", path.display());
    ov.open_document(&uri, &path.to_string_lossy(), 2, "pub fn unused() {}\npub fn used() {}\n".to_string());
    assert_eq!(handle_safe_delete(&at(&path, 0, 8), Some(&index), &ov, &mut conv)["reason"], "staleIndex", "the index is of another text");
    let mut ov = OverlayManager::new();
    let other = path.with_file_name("g.rs");
    ov.open_document(&format!("file://{}", other.display()), &other.to_string_lossy(), 1, "pub fn g() {}\n".to_string());
    ov.apply_content_changes(&other.to_string_lossy(), 2, &[json!({ "text": "pub fn g() { crate::unused() }\n" })]);
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &ov, &mut conv)["reason"], "unsavedUses");

    let lib = "mod m {\n    pub fn child() {}\n}\n";
    let (_d, path, index) = workspace(lib, &[("m", DefKind::Mod, 1, 5), ("child", DefKind::Fn, 2, 12)], &[(3, 5, 1)]);
    let got = handle_safe_delete(&at(&path, 0, 4), Some(&index), &OverlayManager::new(), &mut conv);
    assert_eq!(got["reason"], "used", "its child is used: {got}");

    // Review of f4f8b604: a closed caller changed on disk, unreported, is not what the index read.
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)], &[]);
    assert!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), &mut conv)["edit"].is_object());
    std::fs::write(path.with_file_name("other.rs"), "pub fn caller() { crate::unused() }\n").unwrap();
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), &mut conv)["reason"], "staleIndex");
    // A byte order mark is no column to the compiler.
    let lib = "\u{feff}pub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("unused", DefKind::Fn, 1, 8)], &[]);
    let got = handle_safe_delete(&at(&path, 0, 8), Some(&index), &OverlayManager::new(), &mut conv);
    assert!(got["edit"].is_object(), "{got}");

    let lib = "pub fn a\u{0301}b() {}\n";
    let (_d, path, _index) = workspace(lib, &[], &[]);
    assert_eq!(handle_safe_delete(&at(&path, 0, 9), None, &OverlayManager::new(), &mut conv)["reason"], "unknownSymbol");
}

/// Review of c5dee55b: a mention another configuration compiles, a format string's capture, a macro called by a
/// rename, an included input, a source that cannot be read, a linked or a declared hidden target, a Cargo failure since
/// mended, and ridx's own copies wherever Cargo builds.
#[test]
fn test_what_the_content_check_reads() {
    let lib = "pub fn used() {}\npub fn unused() {}\nmacro_rules! call_it {\n    () => {\n        $crate::unused()\n    };\n}\n";
    let defs = [("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)];
    let mut conv = PositionConverter::new();
    let ask = |path: &std::path::Path, index: &WorkspaceIndex, conv: &mut PositionConverter| {
        let got = handle_safe_delete(&at(path, 1, 8), Some(index), &OverlayManager::new(), conv);
        got["reason"].as_str().map_or("deleted".to_string(), str::to_string)
    };
    let (_d, path, index) = workspace_with(lib, "#[cfg(feature = \"use_it\")]\npub fn caller() { crate::unused() }\n", &defs, &[], false);
    assert_eq!(ask(&path, &index, &mut conv), "staleIndex", "a gated mention the index did not compile");
    let fresh = |files: &[(&str, &str)], conv: &mut PositionConverter| {
        let (d, path, index) = workspace(lib, &defs, &[]);
        let src = path.parent().unwrap().to_path_buf();
        for (name, text) in files {
            let f = src.join(name);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        (ask(&path, &index, conv), d, path, index)
    };
    assert_eq!(fresh(&[("fmt.rs", "pub fn c() { println!(\"{unused}\"); }\n")], &mut conv).0, "staleIndex");
    assert_eq!(fresh(&[("run.rs", "use crate::call_it as run;\npub fn c() { run!(); }\n")], &mut conv).0, "staleIndex");
    assert_eq!(fresh(&[("inc.rs", "pub fn c() { include!(\"body.inc\") }\n"), ("body.inc", "crate::unused()\n")], &mut conv).0, "staleIndex");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (_, d, path, index) = fresh(&[("locked.rs", "pub fn c() {}\n")], &mut conv);
        let locked = path.with_file_name("locked.rs");
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::read(&locked).is_err() {
            assert_eq!(ask(&path, &index, &mut conv), "staleIndex", "a source that cannot be read");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o644)).unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(outside.path().join("caller.rs"), "pub fn c() { crate::unused() }\n").unwrap();
        std::os::unix::fs::symlink(outside.path().join("caller.rs"), path.with_file_name("link.rs")).unwrap();
        assert_eq!(ask(&path, &index, &mut conv), "staleIndex", "a linked source");
        drop(d);
    }
    // A declared hidden target, and a member Cargo could not read yet.
    let (_, _d, path, index) = fresh(&[], &mut conv);
    let root = path.parent().unwrap().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(root.join(".cases")).unwrap();
    std::fs::write(root.join(".cases/caller.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n\n[[test]]\nname = \"caller\"\npath = \".cases/caller.rs\"\n").unwrap();
    assert_eq!(ask(&path, &index, &mut conv), "deleted");
    std::fs::write(root.join(".cases/caller.rs"), "fn main() { w::unused() }\n").unwrap();
    assert_eq!(ask(&path, &index, &mut conv), "staleIndex", "a declared hidden target");
    std::fs::write(root.join(".cases/caller.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n\n[workspace]\nmembers = [\"m\"]\n").unwrap();
    assert_eq!(ask(&path, &index, &mut conv), "staleIndex", "a member Cargo cannot read");
    std::fs::create_dir_all(root.join("m/src")).unwrap();
    std::fs::write(root.join("m/Cargo.toml"), "[package]\nname = \"m\"\nversion = \"0.1.0\"\n").unwrap();
    std::fs::write(root.join("m/src/lib.rs"), "").unwrap();
    assert_eq!(ask(&path, &index, &mut conv), "deleted", "read again once mended");
    // Cargo building elsewhere: ridx's copies under target/ridx are still no sources.
    let elsewhere = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(root.join(".cargo")).unwrap();
    std::fs::write(root.join(".cargo/config.toml"), format!("[build]\ntarget-dir = \"{}\"\n", elsewhere.path().display())).unwrap();
    std::fs::create_dir_all(root.join("target/ridx/sources")).unwrap();
    std::fs::write(root.join("target/ridx/sources/0.rs"), "pub fn c() { crate::unused() }\n").unwrap();
    assert_eq!(ask(&path, &index, &mut conv), "deleted", "ridx's copies");
}

/// Review of b0b16e39: a stored reference that records its target's file is a use of that definition only, the same
/// path elsewhere being another one; a use's columns after a byte order mark keep a two-unit character whole.
#[test]
fn test_stored_edges_keep_their_target_and_whole_characters() {
    let lib = "pub fn used() {}\npub fn unused() {}\n";
    let (_d, path, index) = workspace(lib, &[("used", DefKind::Fn, 1, 8)], &[(3, 5, 0)]);
    let file = ridx_format::canonicalize_best_effort(&path);
    assert_eq!(crate::lsp::editing::safe_delete_edges::raw_edges(&index, &file, 1, 8, "used").map(|e| e.len()), Ok(1));
    let elsewhere = path.with_file_name("other.rs");
    assert_eq!(crate::lsp::editing::safe_delete_edges::raw_edges(&index, &elsewhere, 1, 8, "used").map(|e| e.len()), Ok(0), "its target is lib.rs");
    let lib = "pub fn \u{10400}() {}\n";
    let caller = "\u{feff}pub fn c() { crate::\u{10400}() }\n";
    let (_d, path, index) = workspace_with(lib, caller, &[("\u{10400}", DefKind::Fn, 1, 8)], &[(1, 21, 0)], false);
    let got = handle_safe_delete(&at(&path, 0, 8), Some(&index), &OverlayManager::new(), &mut PositionConverter::new());
    assert_eq!(got["remainingUses"][0]["range"], json!({ "start": { "line": 0, "character": 21 }, "end": { "line": 0, "character": 23 } }), "{got}");
}

/// Review of 53031a6e: an include whose path is computed, a decoded include path, data includes, macros gated or
/// expanding under `cfg`, a name in any string, a hidden directory above a declared target, a link whose target cannot
/// be searched.
#[test]
fn test_includes_macros_strings_and_reachability() {
    let lib = "pub fn used() {}\npub fn unused() {}\nmacro_rules! call_it {\n    () => {\n        $crate::unused()\n    };\n}\nmacro_rules! gen {\n    () => {\n        #[cfg(feature = \"x\")]\n        fn g() { $crate::unused() }\n    };\n}\n";
    let defs = [("used", DefKind::Fn, 1, 8), ("unused", DefKind::Fn, 2, 8)];
    let mut conv = PositionConverter::new();
    let ask = |caller: &str, files: &[(&str, &str)], conv: &mut PositionConverter| {
        let (_d, path, index) = workspace_with(lib, caller, &defs, &[], false);
        let src = path.parent().unwrap().to_path_buf();
        for (name, text) in files {
            let f = src.join(name);
            std::fs::create_dir_all(f.parent().unwrap()).unwrap();
            std::fs::write(f, text).unwrap();
        }
        let got = handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), conv);
        got["reason"].as_str().map_or("deleted".to_string(), str::to_string)
    };
    let plain = "// a caller\n\npub fn caller() {}\n";
    assert_eq!(ask(plain, &[("inc.rs", "pub fn c() { include!(concat!(\"a\", \".inc\")) }\n")], &mut conv), "staleIndex", "a computed include");
    assert_eq!(ask(plain, &[("inc.rs", "pub fn c() -> u8 { include!(\"body\\u{2e}inc\") }\n"), ("body.inc", "1u8\n")], &mut conv), "deleted", "a decoded path");
    assert_eq!(ask(plain, &[("data.rs", "pub const S: &str = include_str!(\"data.txt\");\n"), ("data.txt", "unused\n")], &mut conv), "deleted", "data");
    assert_eq!(ask("#[cfg(feature = \"x\")]\npub fn c() { call_it!(); }\n", &[], &mut conv), "staleIndex", "a gated call of a naming macro");
    assert_eq!(ask("gen!();\n", &[], &mut conv), "staleIndex", "a macro expanding under cfg");
    assert_eq!(ask(plain, &[("fmt.rs", "pub fn c() { println!(\"{:>unused$}\", 1); }\n")], &mut conv), "staleIndex", "a format count");
    assert_eq!(ask(plain, &[("esc.rs", "pub fn c() { println!(\"\\u{7b}unused}\"); }\n")], &mut conv), "staleIndex", "an escaped capture");
    // A target under a hidden directory deeper than the dot.
    let (_d, path, index) = workspace(lib, &defs, &[]);
    let root = path.parent().unwrap().parent().unwrap().to_path_buf();
    std::fs::create_dir_all(root.join(".cases/deep")).unwrap();
    std::fs::write(root.join(".cases/deep/caller.rs"), "fn main() { w::unused() }\n").unwrap();
    std::fs::write(root.join("Cargo.toml"), "[package]\nname = \"w\"\nversion = \"0.1.0\"\n\n[[test]]\nname = \"caller\"\npath = \".cases/deep/caller.rs\"\n").unwrap();
    assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), &mut conv)["reason"], "staleIndex");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let (_d, path, index) = workspace(lib, &defs, &[]);
        let outside = tempfile::tempdir().unwrap();
        let locked = outside.path().join("locked");
        std::fs::create_dir_all(&locked).unwrap();
        std::fs::write(locked.join("caller.rs"), "pub fn c() {}\n").unwrap();
        std::os::unix::fs::symlink(locked.join("caller.rs"), path.with_file_name("link.rs")).unwrap();
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
        if std::fs::metadata(path.with_file_name("link.rs")).is_err_and(|e| e.kind() == std::io::ErrorKind::PermissionDenied) {
            assert_eq!(handle_safe_delete(&at(&path, 1, 8), Some(&index), &OverlayManager::new(), &mut conv)["reason"], "staleIndex", "a link not searchable");
        }
        std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}

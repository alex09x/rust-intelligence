/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Which definitions of a closed file get a `textDocument/codeLens` item: one shard with one
//! `src/lib.rs`, opened through `WorkspaceIndex::open` on the shard file like a single-root server.

use super::handle_code_lens;
use crate::lsp::protocol::{path_to_uri, PositionConverter};
use crate::overlay::OverlayManager;
use ridx_format::{DefKind, IndexWriter, RawDefinition, WorkspaceIndex};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

static DIR_COUNTER: AtomicU64 = AtomicU64::new(0);
static DEF_ID: AtomicU32 = AtomicU32::new(1);
const KEEP: &str = "pub fn keep() {}\n";

struct Shard {
    dir: PathBuf,
    ws: WorkspaceIndex,
}

impl Drop for Shard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

/// A crate whose `src/lib.rs` holds `text`, indexed with `defs`.
fn shard(text: &str, defs: Vec<RawDefinition>) -> Shard {
    let id = DIR_COUNTER.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("ridx_lens_filter_{}_{}", std::process::id(), id));
    fs::create_dir_all(dir.join("src")).unwrap();
    fs::write(dir.join("src/lib.rs"), text).unwrap();
    let mut writer = IndexWriter::new("lens_crate", "0.1.0");
    for d in defs {
        writer.add_definition(d);
    }
    let shard = dir.join("index.ridx");
    writer
        .write_to(&mut fs::File::create(&shard).unwrap())
        .unwrap();
    let ws = WorkspaceIndex::open(&shard).expect("open the shard");
    Shard { dir, ws }
}

/// A definition of `kind` named `name` on `line`, spanning `[start_col, end_col)` in 1-based
/// char columns.
fn def(kind: DefKind, name: &str, (line, start_col, end_col): (u32, u32, u32)) -> RawDefinition {
    RawDefinition {
        def_id: DEF_ID.fetch_add(1, Ordering::Relaxed),
        kind,
        visibility: 1,
        is_macro_expansion: false,
        name: name.to_string(),
        path: format!("lens_crate::{name}"),
        signature: None,
        file: "src/lib.rs".to_string(),
        start_line: line,
        start_col,
        end_line: line,
        end_col,
    }
}

/// Line and 1-based char columns of the first `needle` in `text`.
fn at(text: &str, needle: &str) -> (u32, u32, u32) {
    for (i, line) in text.split('\n').enumerate() {
        if let Some(byte) = line.find(needle) {
            let col = line[..byte].chars().count() as u32 + 1;
            return (i as u32 + 1, col, col + needle.chars().count() as u32);
        }
    }
    panic!("{needle:?} does not occur in {text:?}");
}

fn lenses(sh: &Shard) -> Value {
    let file = sh.dir.join("src/lib.rs");
    let uri = path_to_uri(file.to_str().unwrap());
    let msg = json!({ "params": { "textDocument": { "uri": &uri } } });
    let mut conv = PositionConverter::new();
    handle_code_lens(&msg, Some(&sh.ws), &OverlayManager::new(), &mut conv)
}

fn names(result: &Value) -> Vec<String> {
    result
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l["data"]["name"].as_str().unwrap().to_string())
        .collect()
}

/// `snippet` is line 2 of the file and spells `name`; a definition of `kind` on that span gets no
/// lens while the `keep` function on line 1 does, so only the kind decides.
fn assert_kind_dropped(kind: DefKind, snippet: &str, name: &str) {
    let text = format!("{KEEP}{snippet}\n");
    let sh = shard(
        &text,
        vec![
            def(DefKind::Fn, "keep", at(&text, "keep")),
            def(kind, name, at(&text, name)),
        ],
    );
    assert_eq!(
        names(&lenses(&sh)),
        vec!["keep"],
        "{kind:?} must not get a lens"
    );
}

#[test]
fn test_code_lens_keeps_every_nameable_kind() {
    let items: [(DefKind, &str, &str); 14] = [
        (DefKind::Mod, "mod inner {}", "inner"),
        (DefKind::Struct, "struct Point;", "Point"),
        (DefKind::Union, "union Bits { a: u8 }", "Bits"),
        (DefKind::Enum, "enum Mode { On }", "Mode"),
        (DefKind::Variant, "enum Flag { Set }", "Set"),
        (DefKind::Trait, "trait Shape {}", "Shape"),
        (DefKind::TyAlias, "type Meters = u32;", "Meters"),
        (DefKind::AssocTy, "type Output;", "Output"),
        (DefKind::Fn, "fn run() {}", "run"),
        (DefKind::AssocFn, "fn method(&self) {}", "method"),
        (DefKind::Const, "const LIMIT: u8 = 1;", "LIMIT"),
        (DefKind::AssocConst, "const DEPTH: u8 = 2;", "DEPTH"),
        (DefKind::Static, "static COUNT: u8 = 0;", "COUNT"),
        (DefKind::Macro, "macro_rules! twice {}", "twice"),
    ];
    let text: String = items
        .iter()
        .map(|(_, line, _)| format!("{line}\n"))
        .collect();
    let defs = items
        .iter()
        .map(|(kind, _, name)| def(*kind, name, at(&text, name)))
        .collect();
    let sh = shard(&text, defs);
    let expected: Vec<String> = items.iter().map(|(_, _, n)| n.to_string()).collect();
    assert_eq!(names(&lenses(&sh)), expected);
}

#[test]
fn test_code_lens_skips_use_items() {
    assert_kind_dropped(DefKind::Use, "use other::name;", "name");
}

#[test]
fn test_code_lens_skips_extern_crate_items() {
    assert_kind_dropped(DefKind::ExternCrate, "extern crate name;", "name");
}

#[test]
fn test_code_lens_skips_type_parameters() {
    assert_kind_dropped(DefKind::TyParam, "fn generic<name>() {}", "name");
}

#[test]
fn test_code_lens_skips_const_parameters() {
    assert_kind_dropped(
        DefKind::ConstParam,
        "fn sized<const name: usize>() {}",
        "name",
    );
}

#[test]
fn test_code_lens_skips_lifetime_parameters() {
    assert_kind_dropped(DefKind::LifetimeParam, "fn borrow<'name>() {}", "'name");
}

#[test]
fn test_code_lens_skips_impl_blocks() {
    assert_kind_dropped(DefKind::Impl, "impl name {}", "name");
}

#[test]
fn test_code_lens_skips_foreign_modules() {
    assert_kind_dropped(DefKind::ForeignMod, "extern \"C\" { fn name(); }", "name");
}

#[test]
fn test_code_lens_skips_foreign_types() {
    assert_kind_dropped(DefKind::ForeignTy, "extern \"C\" { type name; }", "name");
}

#[test]
fn test_code_lens_skips_trait_aliases() {
    assert_kind_dropped(DefKind::TraitAlias, "trait name = Other;", "name");
}

#[test]
fn test_code_lens_skips_struct_constructors() {
    assert_kind_dropped(DefKind::StructCtor, "struct name(u8);", "name");
}

#[test]
fn test_code_lens_skips_variant_constructors() {
    assert_kind_dropped(DefKind::VariantCtor, "enum E { name(u8) }", "name");
}

#[test]
fn test_code_lens_skips_fields() {
    assert_kind_dropped(DefKind::Field, "struct S { name: u8 }", "name");
}

#[test]
fn test_code_lens_skips_locals() {
    assert_kind_dropped(DefKind::Local, "fn local() { let name = 1; }", "name");
}

#[test]
fn test_code_lens_skips_closures() {
    assert_kind_dropped(
        DefKind::Closure,
        "fn closure() { let c = |name| name; }",
        "name",
    );
}

#[test]
fn test_code_lens_skips_anonymous_constants() {
    assert_kind_dropped(DefKind::AnonConst, "fn anon() { [0u8; name]; }", "name");
}

#[test]
fn test_code_lens_skips_inline_constants() {
    assert_kind_dropped(
        DefKind::InlineConst,
        "fn inline() { const { name } }",
        "name",
    );
}

#[test]
fn test_code_lens_skips_opaque_types() {
    assert_kind_dropped(DefKind::OpaqueTy, "fn opaque() -> impl name {}", "name");
}

#[test]
fn test_code_lens_skips_global_asm() {
    assert_kind_dropped(DefKind::GlobalAsm, "global_asm!(\"name\");", "name");
}

#[test]
fn test_code_lens_skips_other_kinds() {
    assert_kind_dropped(DefKind::Other, "// name", "name");
}

#[test]
fn test_code_lens_skips_definitions_without_a_name() {
    let text = format!("{KEEP}mod unnamed {{}}\n");
    let mut nameless = def(DefKind::Mod, "", (2, 1, 1));
    nameless.path = "lens_crate".to_string();
    let sh = shard(
        &text,
        vec![def(DefKind::Fn, "keep", at(&text, "keep")), nameless],
    );
    assert_eq!(names(&lenses(&sh)), vec!["keep"]);
}

#[test]
fn test_code_lens_skips_macro_expanded_definitions_even_when_their_span_spells_the_name() {
    let text = format!("{KEEP}pub fn generated() {{}}\n");
    let mut generated = def(DefKind::Fn, "generated", at(&text, "generated"));
    generated.is_macro_expansion = true;
    let sh = shard(
        &text,
        vec![def(DefKind::Fn, "keep", at(&text, "keep")), generated],
    );
    assert_eq!(names(&lenses(&sh)), vec!["keep"]);
}

#[test]
fn test_code_lens_skips_definitions_whose_range_does_not_spell_their_name() {
    // A proc macro generated `MockOpenOptions` from `struct OpenOptions;` and gave it the span of
    // the user's token, without the macro-expansion flag.
    let text = format!("{KEEP}struct OpenOptions;\n");
    let sh = shard(
        &text,
        vec![
            def(DefKind::Fn, "keep", at(&text, "keep")),
            def(DefKind::Struct, "MockOpenOptions", at(&text, "OpenOptions")),
        ],
    );
    assert_eq!(names(&lenses(&sh)), vec!["keep"]);
}

#[test]
fn test_code_lens_drops_the_lenses_of_a_file_edited_after_indexing() {
    let text = "pub fn alpha() {}\n";
    let sh = shard(text, vec![def(DefKind::Fn, "alpha", at(text, "alpha"))]);
    assert_eq!(names(&lenses(&sh)), vec!["alpha"]);
    // The file changes on disk after indexing: the recorded span now covers other text, and the
    // lens that would point at it is dropped instead of being shown on the wrong characters.
    fs::write(sh.dir.join("src/lib.rs"), "// moved\npub fn alpha() {}\n").unwrap();
    assert_eq!(lenses(&sh), json!([]));
}

#[test]
fn test_code_lens_keeps_raw_identifier_definitions() {
    // rustc names the item `match`; the source spells it `r#match`.
    let text = "pub fn r#match() {}\n";
    let (line, start, end) = at(text, "r#match");
    let sh = shard(text, vec![def(DefKind::Fn, "match", (line, start, end))]);
    let result = lenses(&sh);
    assert_eq!(names(&result), vec!["match"]);
    assert_eq!(result[0]["range"]["start"]["character"], start - 1);
    assert_eq!(result[0]["range"]["end"]["character"], end - 1);
}

#[test]
fn test_code_lens_checks_the_name_in_characters_and_reports_utf16_columns() {
    let prefix = "/* \u{1F980} */ pub fn ";
    let text = format!("{prefix}\u{3B3}\u{3B5}\u{3B9}\u{3AC}() {{}}\n");
    let name = "\u{3B3}\u{3B5}\u{3B9}\u{3AC}";
    let sh = shard(&text, vec![def(DefKind::Fn, name, at(&text, name))]);
    let result = lenses(&sh);
    assert_eq!(names(&result), vec![name]);
    // The crab is one char and two UTF-16 units: the span is checked in chars, the range reported
    // in UTF-16 units.
    let start = prefix.encode_utf16().count() as u32;
    assert_eq!(result[0]["range"]["start"]["character"], start);
    assert_eq!(
        result[0]["range"]["end"]["character"],
        start + name.encode_utf16().count() as u32
    );
}

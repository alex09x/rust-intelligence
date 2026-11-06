/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Smart pointer identification and inner type extraction for scoped completion.

use super::super::scope_cache::workspace_deref_fingerprint;
use super::trait_filter::base_type_path;
use super::ufcs::peel_type_modifiers;
use ridx_format::{DefKind, WorkspaceIndex};
use std::collections::HashSet;
use std::sync::{Arc, OnceLock, RwLock};

type DerefWrapperCache = (u64, Arc<HashSet<String>>);

static INDEXED_DEREF_WRAPPER_CACHE: OnceLock<RwLock<DerefWrapperCache>> = OnceLock::new();

/// Checks whether the owner represents a known standard smart pointer (`Box`, `Arc`, `Rc`).
pub fn is_smart_pointer(owner: &str) -> bool {
    let b = base_type_path(owner);
    let short = b.rsplit("::").next().unwrap_or(b);
    matches!(short, "Box" | "Arc" | "Rc")
}

/// Checks whether the owner is a standard library (`core`/`alloc`/`std`) generic `Deref<Target = T>` wrapper.
pub fn is_generic_deref_wrapper(owner: &str) -> bool {
    let b = base_type_path(owner);
    let short = b.rsplit("::").next().unwrap_or(b);
    matches!(
        short,
        "Box"
            | "Arc"
            | "Rc"
            | "Ref"
            | "RefMut"
            | "Cow"
            | "MutexGuard"
            | "RwLockReadGuard"
            | "RwLockWriteGuard"
            | "MappedMutexGuard"
            | "MappedRwLockReadGuard"
            | "MappedRwLockWriteGuard"
            | "ManuallyDrop"
            | "Pin"
            | "LazyLock"
            | "OnceLock"
            | "AssertUnwindSafe"
    )
}

fn is_concrete_non_trait_target(ws: &WorkspaceIndex, crate_name: &str, reader: &ridx_format::IndexReader<memmap2::Mmap>, target: &str) -> bool {
    if matches!(
        target,
        "str" | "slice" | "array" | "bool" | "char" | "usize" | "u8" | "u16" | "u32" | "u64" | "u128" | "isize" | "i8" | "i16" | "i32" | "i64" | "i128" | "f32" | "f64"
    ) || target.starts_with("core::primitive::")
    {
        return true;
    }
    let kind_opt = reader
        .find_definition_by_path(target)
        .map(|d| DefKind::from(d.kind))
        .or_else(|| ws.find_definition_by_path(target).map(|d| d.kind))
        .or_else(|| ws.find_definition_by_path(&format!("{crate_name}::{target}")).map(|d| d.kind));
    // Only a nominal type is a concrete target: not a type parameter (`Res<T>` derefs to its `T`), nor a derive
    // macro sharing a trait's name (`SystemParam`).
    kind_opt.is_some_and(|k| matches!(k, DefKind::Struct | DefKind::Enum | DefKind::Union | DefKind::TyAlias))
}

fn collect_indexed_generic_deref_wrappers(ws: &WorkspaceIndex) -> Arc<HashSet<String>> {
    let fp = workspace_deref_fingerprint(ws);
    let cache = INDEXED_DEREF_WRAPPER_CACHE.get_or_init(|| RwLock::new((0, Arc::new(HashSet::new()))));
    if let Ok(guard) = cache.read() {
        if guard.0 == fp {
            return Arc::clone(&guard.1);
        }
    }

    let mut wrappers = HashSet::new();
    for reader in ws.readers() {
        let cname = reader.crate_name();
        let mut candidates = Vec::new();
        for de in reader.deref_entries() {
            let tgt = reader.get_str(de.target_type_offset());
            if tgt == "core::ops::deref::Deref" || tgt.ends_with("::Deref") || tgt == "Deref" {
                candidates.push(reader.get_str(de.source_type_offset()));
            }
        }
        let defs = reader.definitions();
        for oe in reader.defs_by_owner() {
            if let Some(d) = defs.get(oe.def_index() as usize) {
                let dk = DefKind::from(d.kind);
                let dn = reader.get_str(d.name_offset());
                if (dk == DefKind::AssocFn && dn == "deref") || (dk == DefKind::AssocTy && dn == "Target") {
                    candidates.push(reader.get_str(oe.owner_path_offset()));
                }
            }
        }
        candidates.sort_unstable();
        candidates.dedup();
        for src in candidates {
            if src.is_empty() {
                continue;
            }
            let targets = reader.find_deref_targets(src);
            if targets.iter().any(|&t| is_concrete_non_trait_target(ws, cname, reader, t)) {
                continue;
            }
            wrappers.insert(src.to_string());
            wrappers.insert(format!("{cname}::{src}"));
            let short = src.rsplit("::").next().unwrap_or(src);
            if !short.is_empty() {
                wrappers.insert(short.to_string());
            }
        }
    }

    let arc = Arc::new(wrappers);
    if let Ok(mut guard) = cache.write() {
        *guard = (fp, Arc::clone(&arc));
    }
    arc
}

/// Checks whether `owner` is a standard or `.ridx`-indexed generic `Deref<Target = T>` wrapper.
pub fn is_indexed_generic_deref_wrapper(ws: &WorkspaceIndex, owner: &str) -> bool {
    if is_generic_deref_wrapper(owner) {
        return true;
    }
    let b = base_type_path(owner);
    let short = b.rsplit("::").next().unwrap_or(b);
    let wrappers = collect_indexed_generic_deref_wrappers(ws);
    wrappers.contains(b) || wrappers.contains(short)
}

/// Extracts the `index`-th non-lifetime type argument from a generic type expression.
pub fn extract_nth_non_lifetime_type_arg(ty_name: &str, index: usize) -> Option<&str> {
    let s = peel_type_modifiers(ty_name);
    let start = s.find('<')?;
    let end = s.rfind('>')?;
    if start >= end {
        return None;
    }
    let inner = s[start + 1..end].trim();
    if inner.is_empty() {
        return None;
    }
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    let mut seg_start = 0usize;
    let mut non_lt_idx = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'<' | b'(' | b'[' => depth += 1,
            b'>' | b')' | b']' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                let seg = inner[seg_start..i].trim();
                if !seg.is_empty() && !seg.starts_with('\'') && !seg.contains('=') {
                    if non_lt_idx == index {
                        return Some(peel_type_modifiers(seg));
                    }
                    non_lt_idx += 1;
                }
                seg_start = i + 1;
            }
            _ => {}
        }
    }
    let seg = inner[seg_start..].trim();
    if !seg.is_empty() && !seg.starts_with('\'') && !seg.contains('=') && non_lt_idx == index {
        return Some(peel_type_modifiers(seg));
    }
    None
}

/// Extracts the inner type argument from a smart pointer type expression (e.g. `Box<dyn Trait>` -> `Trait`).
pub fn extract_smart_pointer_inner_type(ty_name: &str) -> Option<&str> {
    let start = ty_name.find('<')?;
    let end = ty_name.rfind('>')?;
    if start >= end {
        return None;
    }
    let mut inner = ty_name[start + 1..end].trim();
    if let Some(rest) = inner.strip_prefix("dyn ") {
        inner = rest.trim();
    }
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'<' {
            depth += 1;
        } else if b == b'>' {
            depth = depth.saturating_sub(1);
        } else if b == b',' && depth == 0 {
            inner = inner[..i].trim();
            break;
        }
    }
    if inner.is_empty() {
        None
    } else {
        Some(inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_smart_pointer() {
        assert!(is_smart_pointer("Box"));
        assert!(is_smart_pointer("alloc::boxed::Box"));
        assert!(is_smart_pointer("Arc"));
        assert!(is_smart_pointer("alloc::sync::Arc"));
        assert!(is_smart_pointer("Rc"));
        assert!(is_smart_pointer("alloc::rc::Rc"));
        assert!(!is_smart_pointer("Vec"));
        assert!(!is_smart_pointer("Option"));
    }

    #[test]
    fn test_extract_smart_pointer_inner_type() {
        assert_eq!(extract_smart_pointer_inner_type("Box<dyn MyTrait>"), Some("MyTrait"));
        assert_eq!(extract_smart_pointer_inner_type("alloc::sync::Arc<Foo, Global>"), Some("Foo"));
        assert_eq!(extract_smart_pointer_inner_type("Rc<Bar<i32>>"), Some("Bar<i32>"));
        assert_eq!(extract_smart_pointer_inner_type("Box<>"), None);
        assert_eq!(extract_smart_pointer_inner_type("NotGeneric"), None);
    }

    #[test]
    fn test_custom_indexed_generic_deref_wrapper_quantum_latch() {
        use crate::lsp::intelligence::owner_completion::{expand_dot_owners, OwnerCompletionContext};
        use ridx_format::writer::{IndexWriter, RawDefinition};

        let temp_dir = tempfile::tempdir().unwrap();
        let idx_path = temp_dir.path().join("quantum_crate.ridx");
        let src_path = temp_dir.path().join("src/lib.rs");
        std::fs::create_dir_all(src_path.parent().unwrap()).unwrap();
        std::fs::write(&src_path, "pub struct QuantumLatch<'a, T>(&'a T);\npub struct TelemetryCore;\n").unwrap();
        let src_str = src_path.to_str().unwrap();

        let mut writer = IndexWriter::new("quantum_crate", "0.1.0");
        writer.add_definition(RawDefinition {
            def_id: 1,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "QuantumLatch".to_string(),
            path: "quantum_crate::sync::QuantumLatch".to_string(),
            file: src_str.to_string(),
            start_line: 1,
            start_col: 12,
            end_line: 1,
            end_col: 24,
            signature: Some("pub struct QuantumLatch<'a, T>".to_string()),
        });
        writer.add_definition(RawDefinition {
            def_id: 2,
            kind: DefKind::Struct,
            visibility: 0,
            is_macro_expansion: false,
            name: "TelemetryCore".to_string(),
            path: "quantum_crate::TelemetryCore".to_string(),
            file: src_str.to_string(),
            start_line: 2,
            start_col: 12,
            end_line: 2,
            end_col: 25,
            signature: Some("pub struct TelemetryCore".to_string()),
        });
        writer.add_deref("sync::QuantumLatch", "core::ops::deref::Deref");
        let mut f = std::fs::File::create(&idx_path).unwrap();
        writer.write_to(&mut f).unwrap();
        drop(f);

        let ws = WorkspaceIndex::open(temp_dir.path()).unwrap();
        assert!(!is_generic_deref_wrapper("QuantumLatch"));
        assert!(is_indexed_generic_deref_wrapper(&ws, "QuantumLatch"));
        assert!(is_indexed_generic_deref_wrapper(&ws, "quantum_crate::sync::QuantumLatch"));

        let ctx = OwnerCompletionContext {
            file: src_str,
            line_1based: 2,
            line_slice: "",
            prefix: "",
            is_dot: true,
            is_colon: false,
            trimmed_before: "",
            overlay_text: None,
            is_dirty: false,
        };
        let mut file_traits = None;
        let (owners, _) = expand_dot_owners(&ws, &ctx, "QuantumLatch<'a, quantum_crate::TelemetryCore>", "QuantumLatch", &mut file_traits);
        assert!(owners.iter().any(|o| o == "quantum_crate::TelemetryCore"), "expected inner target owner in {owners:?}");
    }
}

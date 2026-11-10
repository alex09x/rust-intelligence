/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Qualifier extraction and path/type-head resolution helpers for [`super::query_path`].

use ridx_syntax::name_res::{
    BindingOrigin, CrateDefMap, CrateId, GlobalDefId, ImplTargetKey, ItemKind as IK, LocalItemId, ModuleId, ModuleTree, PathAnchor, PerNs, PrimTy, ResolvedBinding, VisKind,
};
use ridx_syntax::{NodeId, SyntaxKind as K, SyntaxTree};

use super::v5_db::RidxV5ItemDb;

pub(super) fn extract_colon2_qualifier(src: &str, byte_offset: u32) -> Option<(String, u32)> {
    let bytes = src.as_bytes();
    let mut i = (byte_offset as usize).min(bytes.len());
    while i > 0 && (bytes[i - 1].is_ascii_alphanumeric() || bytes[i - 1] == b'_') {
        i -= 1;
    }
    if i >= 2 && &bytes[i - 2..i] == b"r#" {
        i -= 2;
    }
    while i > 0 && matches!(bytes[i - 1], b' ' | b'\t') {
        i -= 1;
    }
    if i < 2 || &bytes[i - 2..i] != b"::" {
        return None;
    }
    let colon2_start = (i - 2) as u32;
    let mut start = i - 2;
    let mut angle = 0i32;
    while start > 0 {
        let b = bytes[start - 1];
        if b == b'>' {
            angle += 1;
            start -= 1;
        } else if b == b'<' {
            angle -= 1;
            start -= 1;
            if angle == 0 && (start == 0 || bytes[start - 1] != b':') {
                break;
            }
        } else if angle > 0 || b.is_ascii_alphanumeric() || matches!(b, b'_' | b':' | b'#') {
            start -= 1;
        } else {
            break;
        }
    }
    let q = src.get(start..i - 2)?.trim();
    (!q.is_empty()).then(|| (q.to_string(), colon2_start))
}

pub(super) fn parse_ufcs_trait(qual: &str) -> Option<&str> {
    let inner = qual.strip_prefix('<')?.strip_suffix('>')?;
    let (_, tr) = inner.rsplit_once(" as ")?;
    Some(tr.trim())
}

pub(super) fn resolve_qual_pns(db: &RidxV5ItemDb<'_>, dm: &CrateDefMap, mod_tree: &ModuleTree, cur_mod: ModuleId, raw_qual: &str) -> Option<PerNs> {
    let stripped = strip_turbofish(raw_qual);
    let rooted = stripped.starts_with("::");
    let clean = stripped.trim_start_matches("::");
    let segs: Vec<&str> = clean.split("::").map(|s| s.trim().trim_start_matches("r#")).filter(|s| !s.is_empty()).collect();
    if segs.is_empty() {
        return None;
    }
    if segs.len() == 1 && !rooted && !matches!(segs[0], "crate" | "self" | "super") {
        return Some(dm.resolve_name_in_module(mod_tree, cur_mod, segs[0]));
    }
    let (anchor, idx) = if rooted {
        (PathAnchor::ExternRoot, 0)
    } else {
        match segs[0] {
            "crate" => (PathAnchor::CrateRoot, 1),
            "self" => (PathAnchor::SelfMod, 1),
            "super" => {
                let n = segs.iter().take_while(|&&s| s == "super").count();
                (PathAnchor::Super(n as u16), n)
            }
            _ => (PathAnchor::Bare, 0),
        }
    };
    if idx == segs.len() {
        let target_mod = match anchor {
            PathAnchor::CrateRoot => Some(ModuleId::ROOT),
            PathAnchor::SelfMod => Some(cur_mod),
            PathAnchor::Super(n) => nth_parent_mod(mod_tree, cur_mod, n),
            _ => None,
        }?;
        let mdef = dm.scope(target_mod).index_def;
        return Some(PerNs {
            types: Some(ResolvedBinding {
                def: mdef,
                local: LocalItemId::NONE,
                local_mod: target_mod,
                vis_mod: ModuleId::ROOT,
                kind: IK::Mod,
                vis: VisKind::Crate,
                origin: BindingOrigin::Explicit,
                prim_ty: 0,
                ambiguous: false,
            }),
            values: None,
            macros: None,
        });
    }
    Some(dm.resolve_path(db, mod_tree, cur_mod, anchor, &segs[idx..]))
}

pub(super) fn nth_parent_mod(mod_tree: &ModuleTree, mut m: ModuleId, steps: u16) -> Option<ModuleId> {
    for _ in 0..steps {
        let p = mod_tree.module(m).parent;
        if !p.is_some() {
            return None;
        }
        m = p;
    }
    Some(m)
}

pub(super) fn mod_is_within(mod_tree: &ModuleTree, mut child: ModuleId, ancestor: ModuleId) -> bool {
    if !ancestor.is_some() {
        return true;
    }
    for _ in 0..64 {
        if child == ancestor {
            return true;
        }
        let p = mod_tree.module(child).parent;
        if !p.is_some() {
            return false;
        }
        child = p;
    }
    false
}

pub(super) fn strip_turbofish(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for ch in s.chars() {
        if ch == '<' {
            if depth == 0 && out.ends_with("::") {
                out.truncate(out.len() - 2);
            }
            depth += 1;
        } else if ch == '>' {
            depth = (depth - 1).max(0);
        } else if depth == 0 {
            out.push(ch);
        }
    }
    out
}

pub(super) fn find_local_module_id(dm: &CrateDefMap, tb: ResolvedBinding) -> Option<ModuleId> {
    if tb.local_mod.is_some() {
        return Some(tb.local_mod);
    }
    if tb.def == dm.scope(ModuleId::ROOT).index_def && tb.def.is_some() {
        return Some(ModuleId::ROOT);
    }
    for (i, sc) in dm.scopes.iter().enumerate() {
        if tb.def.is_some() && sc.index_def == tb.def {
            return Some(ModuleId(i as u32));
        }
    }
    None
}

pub(super) fn unwrap_binding_to_head(db: &RidxV5ItemDb<'_>, dm: &CrateDefMap, tb: ResolvedBinding) -> ImplTargetKey {
    let raw_gid = if tb.def.is_some() {
        tb.def
    } else if tb.local.is_some() {
        GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | tb.local.0)
    } else {
        GlobalDefId::NONE
    };
    if tb.kind == IK::TypeAlias && raw_gid.is_some() {
        if let Some(t) = dm.resolve_alias_target(db, raw_gid) {
            return t;
        }
    }
    if tb.local.is_some() && !tb.def.is_some() {
        return ImplTargetKey::Adt(GlobalDefId::new(CrateId::LOCAL_OVERLAY, 0x8000_0000 | tb.local.0));
    }
    ImplTargetKey::Adt(tb.def)
}

pub(super) fn find_enclosing_impl_or_trait(st: &SyntaxTree<'_>, off: u32) -> Option<NodeId> {
    let b = off as usize;
    let mut best: Option<(usize, NodeId)> = None;
    for nid in (0..st.nodes.len() as u32).map(NodeId) {
        if matches!(st.kind(nid), K::Impl | K::Trait) {
            let r = st.range(nid);
            if r.start <= b && b <= r.end {
                let len = r.end - r.start;
                if best.is_none_or(|(bl, _)| len <= bl) {
                    best = Some((len, nid));
                }
            }
        }
    }
    best.map(|(_, n)| n)
}

pub(super) fn resolve_enclosing_impl_head(
    db: &RidxV5ItemDb<'_>,
    dm: &CrateDefMap,
    mod_tree: &ModuleTree,
    cur_mod: ModuleId,
    st: &SyntaxTree<'_>,
    impl_node: NodeId,
) -> Option<ImplTargetKey> {
    let ty_node = st.children(impl_node).filter(|&c| st.kind(c).is_type()).last()?;
    let txt = strip_turbofish(st.text(ty_node).trim());
    let head_txt = txt.split('<').next()?.trim();
    if let Some(p) = PrimTy::from_name(head_txt) {
        return Some(ImplTargetKey::Prim(p));
    }
    let pns = resolve_qual_pns(db, dm, mod_tree, cur_mod, head_txt)?;
    pns.types.map(|tb| unwrap_binding_to_head(db, dm, tb))
}

pub(super) fn match_enclosing_type_param(st: &SyntaxTree<'_>, fn_node: NodeId, name: &str) -> Option<u16> {
    let mut idx = 0u16;
    if let Some(imp) = st.ancestors(fn_node).find(|&a| matches!(st.kind(a), K::Impl | K::Trait)) {
        if st.kind(imp) == K::Trait {
            idx += 1;
        }
        if let Some(gpl) = st.child(imp, K::GenericParamList) {
            for ch in st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)) {
                if st.kind(ch) == K::TypeParam && st.child(ch, K::Name).is_some_and(|nn| st.text(nn).trim_start_matches("r#") == name) {
                    return Some(idx);
                }
                idx += 1;
            }
        }
    }
    let gpl = st.child(fn_node, K::GenericParamList)?;
    for ch in st.children(gpl).filter(|&c| matches!(st.kind(c), K::TypeParam | K::ConstParam)) {
        if st.kind(ch) == K::TypeParam && st.child(ch, K::Name).is_some_and(|nn| st.text(nn).trim_start_matches("r#") == name) {
            return Some(idx);
        }
        idx += 1;
    }
    None
}

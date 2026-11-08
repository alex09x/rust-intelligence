/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Flat CST arena (`LocalImplArena`) for `impl` blocks, `trait` associated items, and `type`
//! alias targets on dirty-file overlays and unindexed local crates.

use super::db::{FileId, GlobalDefId, ItemKind, ModuleId, VisKind};
use super::db_impls::{ImplTargetKey, LocalImplId};
use super::per_ns::LocalItemId;
use super::use_tree::PathAnchor;
use crate::sema_fn::norm_ident;
use crate::syntax_kind::SyntaxKind as K;
use crate::token::TokenKind as T;
use crate::tree::{NodeId, SyntaxTree};

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum RawSelfHead {
    #[default]
    Unknown,
    Path {
        anchor: PathAnchor,
        seg_start: u32,
        seg_len: u16,
    },
    DynPath {
        anchor: PathAnchor,
        seg_start: u32,
        seg_len: u16,
    },
    Slice,
    Array,
    Tuple(u16),
    Ref {
        is_mut: bool,
    },
    Ptr {
        is_mut: bool,
    },
    FnPtr {
        arity: u16,
    },
    Never,
}

/// Locally extracted `impl` block (resolved into [`ImplTargetKey`] + `trait_def` after import solving).
#[derive(Clone, Copy, Debug)]
pub struct LocalImplData {
    pub id: LocalImplId,
    pub module: ModuleId,
    pub file: FileId,
    pub index_def: GlobalDefId,
    pub is_negative: bool,
    pub has_trait: bool,
    pub(super) raw_head: RawSelfHead,
    pub(super) trait_anchor: PathAnchor,
    pub(super) trait_seg_start: u32,
    pub(super) trait_seg_len: u16,
    pub(super) gp_start: u32,
    pub(super) gp_len: u16,
    pub assoc_start: u32,
    pub assoc_len: u16,
    pub target: ImplTargetKey,
    pub trait_def: GlobalDefId,
    pub span_start: u32,
    pub span_end: u32,
}

/// Associated `fn`, `const`, or `type` inside a local `impl` or `trait` block.
#[derive(Clone, Copy, Debug)]
pub struct LocalAssocItem {
    pub def: GlobalDefId,
    pub module: ModuleId,
    pub file: FileId,
    pub owner_impl: LocalImplId,
    pub owner_trait: LocalItemId,
    pub(super) name_start: u32,
    pub(super) name_end: u32,
    pub kind: ItemKind,
    pub vis: VisKind,
    pub vis_mod: ModuleId,
    pub has_self_receiver: bool,
    pub span_start: u32,
    pub span_end: u32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct LocalTypeAlias {
    pub item_id: LocalItemId,
    pub module: ModuleId,
    pub gp_start: u32,
    pub gp_len: u16,
    pub raw_head: RawSelfHead,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct RawTraitSuper {
    pub trait_item: LocalItemId,
    pub module: ModuleId,
    pub anchor: PathAnchor,
    pub seg_start: u32,
    pub seg_len: u16,
}

/// Contiguous arena of locally parsed `impl` blocks, `trait` associated items, and `type` aliases.
#[derive(Clone, Debug, Default)]
pub struct LocalImplArena {
    pub(super) strings: String,
    pub(super) segs: Vec<(u32, u32)>,
    pub(super) generic_params: Vec<(u32, u32)>,
    pub impls: Vec<LocalImplData>,
    pub assoc_items: Vec<LocalAssocItem>,
    /// `(trait_local_id, assoc_start, assoc_len)` for locally scanned `trait` definitions.
    pub(super) trait_assocs: Vec<(LocalItemId, u32, u16)>,
    pub(super) raw_trait_supers: Vec<RawTraitSuper>,
    pub(super) local_trait_supers: Vec<(LocalItemId, GlobalDefId)>,
    pub(super) type_aliases: Vec<LocalTypeAlias>,
}

impl LocalImplArena {
    #[inline]
    pub fn assoc_name(&self, item: &LocalAssocItem) -> &str {
        &self.strings[item.name_start as usize..item.name_end as usize]
    }

    #[inline]
    pub fn impl_assocs(&self, imp: &LocalImplData) -> &[LocalAssocItem] {
        let s = imp.assoc_start as usize;
        &self.assoc_items[s..s + imp.assoc_len as usize]
    }

    pub(super) fn push_str(&mut self, s: &str) -> (u32, u32) {
        let start = self.strings.len() as u32;
        self.strings.push_str(s);
        (start, self.strings.len() as u32)
    }

    pub(super) fn push_generic_type_params(&mut self, st: &SyntaxTree<'_>, owner: NodeId) -> (u32, u16) {
        let start = self.generic_params.len() as u32;
        if let Some(gpl) = st.child(owner, K::GenericParamList) {
            for gp in st.children(gpl) {
                if st.kind(gp) == K::TypeParam {
                    if let Some(nn) = st.child(gp, K::Name) {
                        let sp = self.push_str(norm_ident(st.text(nn)));
                        self.generic_params.push(sp);
                    }
                }
            }
        }
        let len = (self.generic_params.len() as u32 - start).min(u16::MAX as u32) as u16;
        (start, len)
    }

    pub(super) fn extract_type_head(&mut self, st: &SyntaxTree<'_>, mut ty_node: NodeId) -> RawSelfHead {
        while st.kind(ty_node) == K::ParenType {
            let Some(inner) = st.children(ty_node).find(|&c| st.kind(c).is_type()) else {
                return RawSelfHead::Unknown;
            };
            ty_node = inner;
        }
        match st.kind(ty_node) {
            K::PathType => match st.child(ty_node, K::Path) {
                Some(p) => {
                    let (anchor, seg_start, seg_len) = self.push_cst_path(st, p);
                    RawSelfHead::Path { anchor, seg_start, seg_len }
                }
                None => RawSelfHead::Unknown,
            },
            K::DynTraitType => match find_first_path_in_dyn(st, ty_node) {
                Some(p) => {
                    let (anchor, seg_start, seg_len) = self.push_cst_path(st, p);
                    RawSelfHead::DynPath { anchor, seg_start, seg_len }
                }
                None => RawSelfHead::Unknown,
            },
            K::RefType => RawSelfHead::Ref { is_mut: head_has_mut_before_inner_ty(st, ty_node) },
            K::PtrType => RawSelfHead::Ptr { is_mut: head_has_mut_before_inner_ty(st, ty_node) },
            K::SliceType => RawSelfHead::Slice,
            K::ArrayType => RawSelfHead::Array,
            K::TupleType => {
                let arity = st.children(ty_node).filter(|&c| st.kind(c).is_type()).count() as u16;
                RawSelfHead::Tuple(arity)
            }
            K::FnPtrType => {
                let arity = st.child(ty_node, K::ParamList).map_or(0, |pl| st.children(pl).filter(|&c| st.kind(c) == K::Param).count() as u16);
                RawSelfHead::FnPtr { arity }
            }
            K::NeverType => RawSelfHead::Never,
            _ => RawSelfHead::Unknown,
        }
    }

    pub(super) fn push_cst_path(&mut self, st: &SyntaxTree<'_>, path_node: NodeId) -> (PathAnchor, u32, u16) {
        let mut anchor = PathAnchor::Bare;
        let toks = st.tokens_of(path_node);
        let mut sig = toks.iter().filter(|t| !t.is_trivia());
        if let (Some(t0), Some(t1)) = (sig.next(), sig.next()) {
            if t0.kind == T::Colon && t1.kind == T::Colon {
                anchor = PathAnchor::ExternRoot;
            }
        }
        let seg_start = self.segs.len() as u32;
        let mut idx = 0usize;
        let mut super_cnt = 0u16;
        for seg_node in st.children(path_node) {
            if st.kind(seg_node) != K::PathSegment {
                continue;
            }
            let Some(seg_tok) = st.tokens_of(seg_node).iter().find(|t| !t.is_trivia()) else { continue };
            match seg_tok.kind {
                T::Crate if idx == 0 && anchor == PathAnchor::Bare => {
                    anchor = PathAnchor::CrateRoot;
                }
                T::SelfKw if idx == 0 && anchor == PathAnchor::Bare => {
                    anchor = PathAnchor::SelfMod;
                }
                T::Super if (idx == 0 && anchor == PathAnchor::Bare) || matches!(anchor, PathAnchor::Super(_)) => {
                    if self.segs.len() as u32 == seg_start {
                        super_cnt += 1;
                        anchor = PathAnchor::Super(super_cnt);
                    }
                }
                T::Ident | T::RawIdent | T::SelfType => {
                    let sp = self.push_str(norm_ident(seg_tok.text(st.src)));
                    self.segs.push(sp);
                }
                _ => return (PathAnchor::Bare, seg_start, 0),
            }
            idx += 1;
        }
        let seg_len = (self.segs.len() as u32 - seg_start).min(u16::MAX as u32) as u16;
        (anchor, seg_start, seg_len)
    }
}

fn find_first_path_in_dyn(st: &SyntaxTree<'_>, dyn_node: NodeId) -> Option<NodeId> {
    for ch in st.children(dyn_node) {
        match st.kind(ch) {
            K::PathType => return st.child(ch, K::Path),
            K::TypeBoundList => {
                for b in st.children(ch) {
                    if st.kind(b) == K::TypeBound {
                        if let Some(p) = st.child(b, K::Path) {
                            return Some(p);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn head_has_mut_before_inner_ty(st: &SyntaxTree<'_>, ty_node: NodeId) -> bool {
    let first_inner_tok = st.children(ty_node).find(|&c| st.kind(c).is_type()).map_or(st.node(ty_node).end, |c| st.node(c).first);
    let start_tok = st.node(ty_node).first;
    st.tokens[start_tok as usize..first_inner_tok as usize].iter().any(|t| t.kind == T::Mut)
}

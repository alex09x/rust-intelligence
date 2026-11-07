/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Compiler-free local name resolution (`name_res`) over [`crate::SyntaxTree`] and [`ItemDb`].
//!
//! Phase 2 modules:
//! - [`db`]: `u32`-keyed identifiers (`CrateId`, `FileId`, `ModuleId`, `GlobalDefId`) and
//!   the [`SourceDb`] / [`ItemDb`] boundaries decoupling `ridx-syntax` from `ridx-format`.
//! - [`cfg`]: zero-allocation `#[cfg(...)]` predicate evaluator (`all`, `any`, `not`, `true`, `false`, flags, key-values).
//! - [`attr`]: outer `#[...]` and inner `#![...]` attribute evaluator with recursive `cfg_attr` unwrapping.
//! - [`mbe_parse`] / [`mbe_expand`]: bounded declarative `macro_rules!` expander for item-position macro wrappers.
//! - [`mod_tree`]: flat arena-backed crate module hierarchy (`mod foo;`, `mod foo { ... }`, `#[path]`, `#[cfg]`, `#[macro_use]`).
//! - [`mem_db`]: fast in-memory [`MemSourceDb`] and [`MemItemDb`] implementations.

pub mod attr;
pub mod cfg;
pub mod db;
pub mod mbe_expand;
pub mod mbe_parse;
pub mod mem_db;
mod mod_builder;
mod mod_macro_scope;
mod mod_resolve;
pub mod mod_tree;

pub use attr::{eval_inner_attrs, eval_outer_attrs, extract_visibility, EffectiveAttrs};
pub use cfg::{decode_str_literal, eval_cfg_tokens};
pub use db::{CrateId, Edition, FileId, GlobalDefId, IndexedDef, IndexedUse, ItemDb, ItemKind, ModuleId, NsMask, SourceDb, VisKind};
pub use mbe_parse::MacroRulesDef;
pub use mem_db::{MemItemDb, MemSourceDb, OwnedDef, OwnedUse};
pub use mod_tree::{join_norm_into, join_norm_path, ModChild, ModDiag, ModuleData, ModuleTree};

/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP editing handlers: rename, formatting, code actions, and commands.

pub mod action_adt;
pub mod action_adt_traits;
pub mod action_assists;
pub mod action_crate_checks;
mod action_default_extract;
pub mod action_default_members;
mod action_default_qualify;
pub mod action_default_resolve;
pub mod action_default_safety;
pub mod action_default_scope;
pub mod action_default_ufcs;
pub mod action_doc_template;
pub mod action_extract_lit;
pub mod action_extract_lit_name;
pub mod action_generics;
pub mod action_generics_closure;
pub mod action_generics_helpers;
pub mod action_generics_sub;
pub mod action_inline_local;
pub mod action_parse;
pub mod action_parse_decl;
pub mod action_parse_items;
pub mod action_parse_lex;
pub mod action_raw_string;
pub mod action_struct_conv;
pub mod action_struct_conv_helpers;
mod action_struct_conv_index;
pub mod actions;
pub mod apply_registry;
pub mod atomic_write;
pub mod commands;
pub mod detached_rename;
pub mod extract;
pub mod format;
pub mod ident;
pub mod linked;
pub mod organize;
pub mod prepare_rename;
pub mod ref_span;
pub mod rename;
pub mod rename_alias_mod;
pub mod rename_special;
pub mod rename_unindexed;

#[cfg(test)]
mod tests;

pub use actions::*;
pub use atomic_write::commit_closed_writes_atomic;
pub use commands::*;
pub use extract::*;
pub use format::*;
pub use ident::*;
pub use linked::*;
pub use organize::*;
pub use ref_span::*;
pub use rename::*;

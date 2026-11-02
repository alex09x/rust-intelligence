/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Explicit refusal values and diagnostic reasons for `textDocument/prepareRename` and `textDocument/rename`.

/// Structured reason why renaming a resolved symbol is refused.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenameRefusal {
    /// Cursor does not resolve to a named identifier.
    EmptySymbol,
    /// Target symbol belongs to an external dependency or sysroot crate.
    ExternalSymbol { name: String },
    /// Target symbol is the `fn main` entrypoint of a Cargo `bin`, `example`, or `harness = false` target.
    CargoMainEntrypoint { target_kind: String, target_name: String },
    /// Target symbol is an associated item of an external trait.
    ExternalTraitItem { name: String, trait_path: String },
    /// Target symbol is re-exported across crates with unmerged alias references.
    UnmergedReexport { path: String },
    /// Target method is invoked via an unqualified call inside a `macro_rules!` body across multiple root owners.
    MacroBodyMultiTargetCoupled { name: String, macro_name: String },
    /// A single identifier token in a macro invocation binds to multiple distinct definitions.
    MacroCallDualBoundToken { name: String },
    /// Target inherent method is synthesized from a string literal in a workspace `proc-macro` crate.
    ProcMacroStringCoupled { name: String, crate_name: String },
    /// An unindexed Cargo target references a method name shared by multiple in-scope owners.
    UnindexedTargetAmbiguous { name: String, file: String },
}

impl RenameRefusal {
    /// Formats the refusal reason for `textDocument/rename`.
    pub fn message(&self) -> String {
        match self {
            Self::EmptySymbol => "No renameable symbol found at this position".to_string(),
            Self::ExternalSymbol { name } => format!("Cannot rename external symbol '{name}'"),
            Self::CargoMainEntrypoint { target_kind, target_name } => {
                format!("Cannot rename Cargo {target_kind} entrypoint function 'main' in target '{target_name}'")
            }
            Self::ExternalTraitItem { name, trait_path } => {
                format!("Cannot rename associated item '{name}' of external trait '{trait_path}'")
            }
            Self::UnmergedReexport { .. } => "symbol is re-exported; cross-crate references may be incomplete".to_string(),
            Self::MacroBodyMultiTargetCoupled { name, macro_name } => {
                format!("Cannot rename symbol '{name}': identifier is hardcoded in macro_rules! '{macro_name}' across multiple owners")
            }
            Self::MacroCallDualBoundToken { name } => {
                format!("Cannot rename symbol '{name}': macro call token binds to multiple definitions")
            }
            Self::ProcMacroStringCoupled { name, crate_name } => {
                format!("Cannot rename symbol '{name}': identifier is synthesized from string literal in proc-macro crate '{crate_name}'")
            }
            Self::UnindexedTargetAmbiguous { name, file } => {
                format!("Cannot rename symbol '{name}': ambiguous unindexed reference in '{file}'")
            }
        }
    }

    /// Formats the refusal reason for `textDocument/prepareRename`.
    pub fn prepare_message(&self) -> String {
        match self {
            Self::EmptySymbol => "Cannot prepare rename: no identifier found at this position".to_string(),
            other => other.message(),
        }
    }
}

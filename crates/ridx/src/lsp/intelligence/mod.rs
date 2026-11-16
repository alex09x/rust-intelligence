/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP intelligence handlers: hover, completion, signature help, and inlay hints.

pub mod auto_import;
pub mod completion;
pub mod completion_call;
pub(crate) mod completion_docs;
pub mod completion_edits;
mod completion_import;
pub mod fallback_completion;
pub(crate) mod fn_params;
pub mod generic_completion;
pub mod hints;
pub mod hover;
pub(crate) mod hover_decl;
pub(crate) mod hover_doc;
pub mod hover_enrich;
pub(crate) mod hover_generics;
pub(crate) mod hover_render;
mod hover_unindexed;
pub mod inlay;
mod live_completion;
pub mod owner_completion;
pub mod scope_cache;
pub mod self_completion;
pub mod signature;
pub(crate) mod signature_fmt;
pub mod syntax_pos;

#[cfg(test)]
pub mod tests;

pub use auto_import::*;
pub use completion::*;
pub use fallback_completion::*;
pub use hints::*;
pub use hover::*;
pub use signature::*;
pub use syntax_pos::*;

/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! Live compiler-free name resolution and type inference adapter over `WorkspaceIndex` (`v5` `.ridx` mmap shards).

pub mod meta;
pub mod query;
mod query_dot;
mod query_loc;
mod query_path;
mod query_path_qual;
mod query_state;
mod query_ty_fmt;
pub mod v5_db;
pub mod v5_db_queries;
pub mod v5_lazy;
pub mod v5_macro_res;
pub mod v5_shard;
pub mod v5_trait_solve;
pub mod v5_ty_bounds;
pub mod v5_ty_db;
pub mod v5_ty_defaults;
pub mod v5_ty_inst;
pub mod v5_ty_lower;
pub mod v5_ty_proj;
pub mod v5_ty_tables;

#[cfg(test)]
mod query_tests;

pub use meta::{load_reader_metas, CorpusFile, CorpusSourceDb, LineIndex, ReaderMeta};
pub use query::{Cand, LiveDb, LiveLocation, LiveTy};
pub use v5_db::{RidxV5ItemDb, CRATE_ROOT_DEF_IDX, IMPL_SYNTH_BIT, PROC_MACRO_SYNTH_BIT, TY_DEF_SYNTH_BIT};
pub use v5_ty_lower::{lower_v5_ty, ParamRemap};

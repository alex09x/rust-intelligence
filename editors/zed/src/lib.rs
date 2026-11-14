/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! rust-intelligence for Zed: `ridx lsp` as the Rust language server, under its own name.
//!
//! The server is found on the worktree's `PATH` (next to `ridx-driver`, which indexing runs), or at
//! `lsp.rust-intelligence.binary.path`. Its `initialization_options` and `settings` are its own, or else
//! those written for rust-analyzer, whose options `ridx lsp` reads.

use zed_extension_api::{self as zed, serde_json, settings::LspSettings, LanguageServerId, Result};

struct RustIntelligence;

/// The settings of this server, or else rust-analyzer's.
fn settings(server: &LanguageServerId, worktree: &zed::Worktree) -> (Option<LspSettings>, Option<LspSettings>) {
    (LspSettings::for_worktree(server.as_ref(), worktree).ok(), LspSettings::for_worktree("rust-analyzer", worktree).ok())
}

impl zed::Extension for RustIntelligence {
    fn new() -> Self {
        RustIntelligence
    }

    fn language_server_command(&mut self, server: &LanguageServerId, worktree: &zed::Worktree) -> Result<zed::Command> {
        let binary = LspSettings::for_worktree(server.as_ref(), worktree).ok().and_then(|s| s.binary);
        let command = binary.as_ref().and_then(|b| b.path.clone()).or_else(|| worktree.which("ridx")).ok_or_else(|| {
            "ridx is not on the PATH: install it with `cargo install --path crates/ridx --locked` and \
             `cargo install --path crates/ridx-driver --locked` from https://github.com/alex09x/rust-intelligence, \
             or set lsp.rust-intelligence.binary.path in the settings"
                .to_string()
        })?;
        let args = binary.as_ref().and_then(|b| b.arguments.clone()).unwrap_or_else(|| vec!["lsp".to_string()]);
        let mut env = worktree.shell_env();
        if let Some(extra) = binary.and_then(|b| b.env) {
            env.extend(extra);
        }
        Ok(zed::Command { command, args, env })
    }

    fn language_server_initialization_options(&mut self, server: &LanguageServerId, worktree: &zed::Worktree) -> Result<Option<serde_json::Value>> {
        let (own, native) = settings(server, worktree);
        Ok(own.and_then(|s| s.initialization_options).or_else(|| native.and_then(|s| s.initialization_options)))
    }

    fn language_server_workspace_configuration(&mut self, server: &LanguageServerId, worktree: &zed::Worktree) -> Result<Option<serde_json::Value>> {
        let (own, native) = settings(server, worktree);
        Ok(own.and_then(|s| s.settings).or_else(|| native.and_then(|s| s.settings)))
    }
}

zed::register_extension!(RustIntelligence);

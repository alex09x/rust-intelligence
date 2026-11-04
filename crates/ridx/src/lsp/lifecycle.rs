/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

//! LSP server lifecycle state machine, capability advertisement, and shutdown handling.

use crate::lsp::editing::apply_registry::ApplyEditRegistry;
pub use crate::lsp::editing::apply_registry::PendingApplyState;
use crate::lsp::presentation::flycheck::FlycheckState;
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    Uninitialized,
    Initialized,
    ShutDown,
}

pub struct LifecycleManager {
    state: ServerState,
    pub flycheck: Arc<FlycheckState>,
    pub conv: crate::lsp::protocol::PositionConverter,
    client_pull_diagnostics: bool,
    client_refresh_diagnostics: bool,
    client_resolve_code_action_edit: bool,
    client_ra_show_references: bool,
    refresh_counter: AtomicU64,
    pub apply_registry: ApplyEditRegistry,
}

impl Default for LifecycleManager {
    fn default() -> Self {
        Self::new()
    }
}

impl LifecycleManager {
    pub fn new() -> Self {
        Self {
            state: ServerState::Uninitialized,
            flycheck: Arc::new(FlycheckState::new()),
            conv: crate::lsp::protocol::PositionConverter::new(),
            client_pull_diagnostics: false,
            client_refresh_diagnostics: false,
            client_resolve_code_action_edit: false,
            client_ra_show_references: false,
            refresh_counter: AtomicU64::new(0),
            apply_registry: ApplyEditRegistry::new(),
        }
    }

    pub fn client_pull_diagnostics(&self) -> bool {
        self.client_pull_diagnostics
    }

    pub fn client_refresh_diagnostics(&self) -> bool {
        self.client_refresh_diagnostics
    }

    pub fn client_resolve_code_action_edit(&self) -> bool {
        self.client_resolve_code_action_edit
    }

    /// The client runs `rust-analyzer.showReferences` itself, as the rust-analyzer VS Code extension and
    /// rustaceanvim announce in `experimental.commands.commands`; lenses then use that command name.
    pub fn client_ra_show_references(&self) -> bool {
        self.client_ra_show_references
    }

    pub fn next_refresh_id(&self) -> String {
        let n = self.refresh_counter.fetch_add(1, Ordering::SeqCst) + 1;
        format!("ridx-refresh-{}", n)
    }

    pub fn get_next_apply_id(&self) -> Value {
        self.apply_registry.get_next_id()
    }

    pub fn register_pending_apply(&self, apply_req_id: Value, command_req_id: Option<Value>, total_count: usize, combined_edit: Value) {
        self.apply_registry.register_pending(apply_req_id, command_req_id, total_count, combined_edit);
    }

    pub fn handle_apply_edit_response(&self, msg: &Value) -> Option<(Option<Value>, Value)> {
        self.apply_registry.handle_response(msg)
    }

    #[allow(dead_code)]
    pub fn state(&self) -> ServerState {
        self.state
    }

    pub fn is_initialized(&self) -> bool {
        self.state == ServerState::Initialized
    }

    pub fn is_shutdown(&self) -> bool {
        self.state == ServerState::ShutDown
    }

    pub fn handle_initialize(&mut self, msg: &Value) -> Result<(Value, Vec<PathBuf>), super::error::LspError> {
        if self.state != ServerState::Uninitialized {
            return Err(super::error::LspError::invalid_request("Server is already initialized"));
        }

        self.state = ServerState::Initialized;
        self.client_pull_diagnostics = msg["params"]["capabilities"]["textDocument"]["diagnostic"].is_object();
        self.client_refresh_diagnostics = msg["params"]["capabilities"]["workspace"]["diagnostics"]["refreshSupport"].as_bool().unwrap_or(false);
        self.client_resolve_code_action_edit = msg["params"]["capabilities"]["textDocument"]["codeAction"]["resolveSupport"]["properties"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("edit")));
        self.client_ra_show_references = msg["params"]["capabilities"]["experimental"]["commands"]["commands"]
            .as_array()
            .is_some_and(|arr| arr.iter().any(|v| v.as_str() == Some("rust-analyzer.showReferences")));

        let mut folder_paths = Vec::new();
        if let Some(folders) = msg["params"]["workspaceFolders"].as_array() {
            for folder in folders {
                if let Some(uri) = folder["uri"].as_str() {
                    let path = super::protocol::uri_to_path(uri);
                    folder_paths.push(PathBuf::from(path));
                }
            }
        }

        if folder_paths.is_empty() {
            if let Some(root_uri) = msg["params"]["rootUri"].as_str() {
                let path = super::protocol::uri_to_path(root_uri);
                folder_paths.push(PathBuf::from(path));
            } else if let Some(root_path) = msg["params"]["rootPath"].as_str() {
                folder_paths.push(PathBuf::from(root_path));
            }
        }

        let rs_filter = json!([{ "pattern": { "glob": "**/*.rs" } }]);
        let file_operations = json!({
            "didCreate": { "filters": rs_filter },
            "willCreate": { "filters": rs_filter },
            "didRename": { "filters": rs_filter },
            "willRename": { "filters": rs_filter },
            "didDelete": { "filters": rs_filter },
            "willDelete": { "filters": rs_filter }
        });

        let mut caps = json!({
            "textDocumentSync": {
                "openClose": true,
                "change": 2, // Incremental
                "willSave": true,
                "willSaveWaitUntil": true,
                "save": { "includeText": false }
            },
            "definitionProvider": true,
            "declarationProvider": true,
            "typeDefinitionProvider": true,
            "implementationProvider": true,
            "referencesProvider": true,
            "documentHighlightProvider": true,
            "documentSymbolProvider": true,
            "workspaceSymbolProvider": { "resolveProvider": true },
            "hoverProvider": true,
            "completionProvider": {
                "triggerCharacters": [".", ":"],
                "resolveProvider": true
            },
            "signatureHelpProvider": {
                "triggerCharacters": ["(", ","]
            },
            "documentFormattingProvider": true,
            "documentRangeFormattingProvider": true,
            "documentOnTypeFormattingProvider": {
                "firstTriggerCharacter": ";",
                "moreTriggerCharacter": ["}"]
            },
            "renameProvider": { "prepareProvider": true },
            "linkedEditingRangeProvider": true,
            "callHierarchyProvider": true,
            "typeHierarchyProvider": true,
            "documentLinkProvider": { "resolveProvider": true },
            "documentColorProvider": true,
            "selectionRangeProvider": true,
            "foldingRangeProvider": true,
            "codeActionProvider": {
                "codeActionKinds": ["quickfix", "refactor", "source.organizeImports"],
                "resolveProvider": true
            },
            "codeLensProvider": { "resolveProvider": true },
            "executeCommandProvider": {
                "commands": [
                    "ridx.showReferences",
                    "ridx.applyEdits",
                    "ridx.resolveWorkspace"
                ]
            },
            "inlayHintProvider": { "resolveProvider": true },
            "inlineValueProvider": true,
            "monikerProvider": true,
            "workspace": {
                "workspaceFolders": {
                    "supported": true,
                    "changeNotifications": true
                },
                "fileOperations": file_operations
            }
        });

        caps["semanticTokensProvider"] = json!({
            "legend": {
                "tokenTypes": [
                    "namespace", "type", "class", "enum", "interface",
                    "struct", "typeParameter", "parameter", "variable",
                    "property", "enumMember", "function", "method",
                    "macro", "keyword", "comment", "string", "number"
                ],
                "tokenModifiers": ["declaration", "definition", "readonly", "static"]
            },
            "range": true,
            "full": { "delta": true }
        });

        caps["diagnosticProvider"] = json!({
            "interFileDependencies": true,
            "workspaceDiagnostics": true,
        });

        let capabilities = json!({
            "capabilities": caps,
            "serverInfo": {
                "name": "ridx",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        Ok((capabilities, folder_paths))
    }

    pub fn handle_shutdown(&mut self) -> Result<Value, super::error::LspError> {
        if self.state == ServerState::ShutDown {
            return Err(super::error::LspError::invalid_request("Server is already shut down"));
        }
        self.state = ServerState::ShutDown;
        self.flycheck.shutdown();
        self.apply_registry.clear();
        super::hierarchy::clear_impl_table_cache();
        Ok(Value::Null)
    }

    pub fn handle_exit(&self) -> Result<(), std::io::Error> {
        if self.state == ServerState::ShutDown {
            Ok(())
        } else {
            Err(std::io::Error::other("exit notification received before shutdown"))
        }
    }

    pub fn check_lifecycle_and_cancel<W: std::io::Write>(
        &mut self,
        method: &str,
        id: &Option<Value>,
        cancelled_requests: &std::sync::Arc<std::sync::Mutex<std::collections::HashSet<Value>>>,
        writer: &mut W,
    ) -> Result<Option<bool>, std::io::Error> {
        if method == "exit" {
            return self.handle_exit().map(|_| Some(false)).map_err(|_| std::io::Error::other("exit notification received before shutdown"));
        }
        let req_id = match id {
            Some(r) => r,
            None => return Ok(None),
        };
        if self.is_shutdown() {
            super::protocol::send_response(writer, &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32600, "message": "Server is shutting down" } }))?;
            return Ok(Some(true));
        }
        if !self.is_initialized() && method != "initialize" {
            super::protocol::send_response(writer, &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32002, "message": "Server not initialized" } }))?;
            return Ok(Some(true));
        }
        if cancelled_requests.lock().unwrap().remove(req_id) {
            let _ = super::protocol::send_response(writer, &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32800, "message": "Request cancelled" } }));
            return Ok(Some(true));
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_lifecycle_transitions() {
        let mut lm = LifecycleManager::new();
        assert_eq!(lm.state(), ServerState::Uninitialized);
        assert!(!lm.is_initialized());

        let init_msg = json!({
            "params": {
                "rootUri": "file:///workspace",
                "workspaceFolders": [
                    { "uri": "file:///workspace1" },
                    { "uri": "file:///workspace2" }
                ]
            }
        });

        let (res, folders) = lm.handle_initialize(&init_msg).unwrap();
        assert!(res["capabilities"]["definitionProvider"].as_bool().unwrap());
        assert_eq!(folders.len(), 2);
        assert_eq!(lm.state(), ServerState::Initialized);
        assert!(lm.is_initialized());

        // Initialize with rootUri when workspaceFolders is missing (lines 68-70)
        let mut lm_uri = LifecycleManager::new();
        let init_uri = json!({
            "params": {
                "rootUri": "file:///workspace_uri"
            }
        });
        let (_, folders_uri) = lm_uri.handle_initialize(&init_uri).unwrap();
        assert_eq!(folders_uri.len(), 1);

        // Initialize with rootPath when workspaceFolders and rootUri are missing (lines 71-73)
        let mut lm_path = LifecycleManager::new();
        let init_path = json!({
            "params": {
                "rootPath": "/workspace_path"
            }
        });
        let (_, folders_path) = lm_path.handle_initialize(&init_path).unwrap();
        assert_eq!(folders_path.len(), 1);

        // Repeated initialize is rejected
        let second_init = lm.handle_initialize(&init_msg);
        assert!(second_init.is_err());
        assert_eq!(second_init.unwrap_err().code, -32600);

        // Exit before shutdown errors
        assert!(lm.handle_exit().is_err());

        // Shutdown transitions to ShutDown
        let shut_res = lm.handle_shutdown().unwrap();
        assert!(shut_res.is_null());
        assert_eq!(lm.state(), ServerState::ShutDown);
        assert!(lm.is_shutdown());

        // Exit after shutdown succeeds
        assert!(lm.handle_exit().is_ok());
    }

    #[test]
    fn test_apply_edit_counter_and_pending_applies_per_instance() {
        let (lm1, lm2) = (LifecycleManager::new(), LifecycleManager::new());
        assert_eq!(lm1.get_next_apply_id(), json!(10000));
        assert_eq!(lm2.get_next_apply_id(), json!(10000));
        assert_eq!(lm1.get_next_apply_id(), json!(10001));
        assert_eq!(lm2.get_next_apply_id(), json!(10001));

        lm1.register_pending_apply(json!(10000), Some(json!(404)), 2, json!({"test": 1}));
        let ack = json!({ "id": 10000, "result": { "applied": true } });
        assert_eq!(lm2.handle_apply_edit_response(&ack), None);
        assert_eq!(lm1.handle_apply_edit_response(&ack), Some((Some(json!(404)), json!({ "applied": true, "count": 2, "edit": { "test": 1 } }))));
    }
}

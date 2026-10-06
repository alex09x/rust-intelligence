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

use crate::lsp::presentation::flycheck::FlycheckState;
use serde_json::{json, Value};
use std::path::PathBuf;
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
        }
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

    pub fn handle_initialize(
        &mut self,
        msg: &Value,
    ) -> Result<(Value, Vec<PathBuf>), (i32, String)> {
        if self.state != ServerState::Uninitialized {
            return Err((-32600, "Server is already initialized".to_string()));
        }

        self.state = ServerState::Initialized;

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

        let capabilities = json!({
            "capabilities": {
                "textDocumentSync": {
                    "openClose": true,
                    "change": 2, // Incremental
                    "save": { "includeText": false }
                },
                "definitionProvider": true,
                "declarationProvider": true,
                "typeDefinitionProvider": true,
                "implementationProvider": true,
                "referencesProvider": true,
                "documentHighlightProvider": true,
                "documentSymbolProvider": true,
                "workspaceSymbolProvider": {
                    "resolveProvider": true
                },
                "hoverProvider": true,
                "completionProvider": {
                    "triggerCharacters": [".", ":"],
                    "resolveProvider": true
                },
                "signatureHelpProvider": {
                    "triggerCharacters": ["(", ","]
                },
                "documentFormattingProvider": true,
                "renameProvider": {
                    "prepareProvider": true
                },
                "callHierarchyProvider": true,
                "typeHierarchyProvider": true,
                "semanticTokensProvider": {
                    "legend": {
                        "tokenTypes": [
                            "namespace", "type", "class", "enum", "interface",
                            "struct", "typeParameter", "parameter", "variable",
                            "property", "enumMember", "function", "method",
                            "macro", "keyword", "comment", "string", "number"
                        ],
                        "tokenModifiers": ["declaration", "definition", "readonly", "static"]
                    },
                    "full": true
                },
                "documentLinkProvider": {
                    "resolveProvider": true
                },
                "selectionRangeProvider": true,
                "foldingRangeProvider": true,
                "codeActionProvider": {
                    "codeActionKinds": ["quickfix", "refactor", "source.organizeImports"],
                    "resolveProvider": true
                },
                "codeLensProvider": {
                    "resolveProvider": true
                },
                "inlayHintProvider": {
                    "resolveProvider": false
                },
                "workspace": {
                    "workspaceFolders": {
                        "supported": true,
                        "changeNotifications": true
                    },
                    "fileOperations": {
                        "didRename": {
                            "filters": [{ "pattern": { "glob": "**/*.rs" } }]
                        },
                        "didDelete": {
                            "filters": [{ "pattern": { "glob": "**/*.rs" } }]
                        }
                    }
                }
            },
            "serverInfo": {
                "name": "ridx",
                "version": "0.1.0"
            }
        });

        Ok((capabilities, folder_paths))
    }

    pub fn handle_shutdown(&mut self) -> Result<Value, (i32, String)> {
        if self.state == ServerState::ShutDown {
            return Err((-32600, "Server is already shut down".to_string()));
        }
        self.state = ServerState::ShutDown;
        self.flycheck.shutdown();
        Ok(Value::Null)
    }

    pub fn handle_exit(&self) -> Result<(), std::io::Error> {
        if self.state == ServerState::ShutDown {
            Ok(())
        } else {
            Err(std::io::Error::other(
                "exit notification received before shutdown",
            ))
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
            return self
                .handle_exit()
                .map(|_| Some(false))
                .map_err(|_| std::io::Error::other("exit notification received before shutdown"));
        }
        let req_id = match id {
            Some(r) => r,
            None => return Ok(None),
        };
        if self.is_shutdown() {
            super::protocol::send_response(
                writer,
                &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32600, "message": "Server is shutting down" } }),
            )?;
            return Ok(Some(true));
        }
        if !self.is_initialized() && method != "initialize" {
            super::protocol::send_response(
                writer,
                &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32002, "message": "Server not initialized" } }),
            )?;
            return Ok(Some(true));
        }
        if cancelled_requests.lock().unwrap().remove(req_id) {
            let _ = super::protocol::send_response(
                writer,
                &json!({ "jsonrpc": "2.0", "id": req_id, "error": { "code": -32800, "message": "Request cancelled" } }),
            );
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
        assert_eq!(second_init.unwrap_err().0, -32600);

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
}

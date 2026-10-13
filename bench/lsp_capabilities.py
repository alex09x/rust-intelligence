# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Client capabilities the benchmark harness announces in `initialize`, identical for both servers.
"""

CLIENT_CAPABILITIES = {
    "window": {"workDoneProgress": True},
    "workspace": {
        "configuration": True,
        "workspaceFolders": True,
        "applyEdit": True,
        "symbol": {"dynamicRegistration": True},
    },
    "textDocument": {
        "synchronization": {"dynamicRegistration": True, "willSave": True, "willSaveWaitUntil": True, "didSave": True},
        "definition": {"dynamicRegistration": True, "linkSupport": True},
        "references": {"dynamicRegistration": True},
        "hover": {"dynamicRegistration": True, "contentFormat": ["markdown", "plaintext"]},
        "completion": {"dynamicRegistration": True, "completionItem": {"snippetSupport": True}},
        "signatureHelp": {"dynamicRegistration": True},
        "documentSymbol": {"dynamicRegistration": True, "hierarchicalDocumentSymbolSupport": True},
        "semanticTokens": {
            "dynamicRegistration": True,
            "tokenTypes": [
                "comment", "keyword", "string", "number", "regexp", "operator", "namespace",
                "type", "struct", "class", "interface", "enum", "typeParameter", "function",
                "method", "decorator", "macro", "variable", "parameter", "property", "label",
            ],
            "tokenModifiers": [
                "declaration", "definition", "readonly", "static", "deprecated",
                "abstract", "async", "modification", "documentation", "defaultLibrary",
            ],
            "requests": {"full": True, "range": True},
            "formats": ["relative"],
        },
        "inlayHint": {"dynamicRegistration": True},
        "codeAction": {"dynamicRegistration": True},
        "formatting": {"dynamicRegistration": True},
    },
    "experimental": {"serverStatusNotification": True},
}

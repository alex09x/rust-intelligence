# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
Client capabilities the benchmark harness announces in `initialize`, identical for both servers.

`experimental.commands.commands` lists the client commands rust-analyzer checks before it builds a
code lens (rust-lang/rust 48a229cea, `src/tools/rust-analyzer/crates/rust-analyzer/src/config.rs`,
`client_commands`): without `rust-analyzer.showReferences` it emits no implementations or
references lens, without `rust-analyzer.runSingle` / `rust-analyzer.debugSingle` no Run or Debug
lens (`lsp/to_proto.rs`, `code_lens`). The three other commands it knows are left out on purpose:
`rust-analyzer.gotoLocation` only serves hover actions, which also need `experimental.hoverActions`
(not declared), `rust-analyzer.triggerParameterHints` adds a command to completion items and code
actions, and `rust-analyzer.rename` adds a command to code actions; declaring them would change
those answers. `workspace.codeLens.refreshSupport` stays undeclared so the server sends no
`workspace/codeLens/refresh` requests.
"""

RA_CLIENT_COMMANDS = ["rust-analyzer.runSingle", "rust-analyzer.debugSingle", "rust-analyzer.showReferences"]

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
        "codeLens": {"dynamicRegistration": True},
        "formatting": {"dynamicRegistration": True},
    },
    "experimental": {"serverStatusNotification": True, "commands": {"commands": RA_CLIENT_COMMANDS}},
}

# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""
LSP 3.17 method specifications and parameter constructors across 10 functional domains.
Defines all 78 distinct methods, RPC types, and strict schema-valid payload generators.
"""

from pathlib import Path


DOMAIN_NAMES = {
    1: "Lifecycle & Control",
    2: "Document Synchronization",
    3: "Language Intelligence",
    4: "Hierarchies",
    5: "Selection, Links & Colors",
    6: "Editing & Refactoring",
    7: "Presentation & Tokens",
    8: "Diagnostics",
    9: "Workspace & File Operations",
    10: "Memory Mutation Scenarios",
}

# Specification of all 78 methods
# rpc_type: "request" | "notification" | "server_rpc" | "lifecycle" | "mutation"
LSP_SPECS = [
    # Domain 1: Lifecycle & Control (1..7)
    (1, "initialize", "initialize", 1, "lifecycle"),
    (2, "initialized", "initialized", 1, "lifecycle"),
    (3, "shutdown", "shutdown", 1, "lifecycle"),
    (4, "exit", "exit", 1, "lifecycle"),
    (5, "cancel_request", "$/cancelRequest", 1, "notification"),
    (6, "set_trace", "$/setTrace", 1, "notification"),
    (7, "progress_cancel", "window/workDoneProgress/cancel", 1, "notification"),

    # Domain 2: Document Synchronization (8..14)
    (8, "did_open", "textDocument/didOpen", 2, "notification"),
    (9, "did_change_full", "textDocument/didChange", 2, "notification"),
    (10, "did_change_inc", "textDocument/didChange", 2, "notification"),
    (11, "will_save", "textDocument/willSave", 2, "notification"),
    (12, "will_save_wait", "textDocument/willSaveWaitUntil", 2, "request"),
    (13, "did_save", "textDocument/didSave", 2, "notification"),
    (14, "did_close", "textDocument/didClose", 2, "notification"),

    # Domain 3: Language Intelligence (15..33)
    (15, "definition", "textDocument/definition", 3, "request"),
    (16, "declaration", "textDocument/declaration", 3, "request"),
    (17, "type_definition", "textDocument/typeDefinition", 3, "request"),
    (18, "implementation", "textDocument/implementation", 3, "request"),
    (19, "references", "textDocument/references", 3, "request"),
    (20, "document_highlight", "textDocument/documentHighlight", 3, "request"),
    (21, "document_symbol", "textDocument/documentSymbol", 3, "request"),
    (22, "workspace_symbol", "workspace/symbol", 3, "request"),
    (23, "workspace_symbol_resolve", "workspaceSymbol/resolve", 3, "request"),
    (24, "hover", "textDocument/hover", 3, "request"),
    (25, "completion", "textDocument/completion", 3, "request"),
    (26, "completion_resolve", "completionItem/resolve", 3, "request"),
    (27, "signature_help", "textDocument/signatureHelp", 3, "request"),
    (28, "inlay_hint", "textDocument/inlayHint", 3, "request"),
    (29, "inlay_hint_resolve", "inlayHint/resolve", 3, "request"),
    (30, "inline_value", "textDocument/inlineValue", 3, "request"),
    (31, "inline_completion", "textDocument/inlineCompletion", 3, "request"),
    (32, "moniker", "textDocument/moniker", 3, "request"),
    (33, "moniker_resolve", "moniker/resolve", 3, "request"),

    # Domain 4: Hierarchies (34..39)
    (34, "prepare_call_hierarchy", "textDocument/prepareCallHierarchy", 4, "request"),
    (35, "incoming_calls", "callHierarchy/incomingCalls", 4, "request"),
    (36, "outgoing_calls", "callHierarchy/outgoingCalls", 4, "request"),
    (37, "prepare_type_hierarchy", "textDocument/prepareTypeHierarchy", 4, "request"),
    (38, "supertypes", "typeHierarchy/supertypes", 4, "request"),
    (39, "subtypes", "typeHierarchy/subtypes", 4, "request"),

    # Domain 5: Selection, Links & Colors (40..44)
    (40, "selection_range", "textDocument/selectionRange", 5, "request"),
    (41, "document_link", "textDocument/documentLink", 5, "request"),
    (42, "document_link_resolve", "documentLink/resolve", 5, "request"),
    (43, "document_color", "textDocument/documentColor", 5, "request"),
    (44, "color_presentation", "textDocument/colorPresentation", 5, "request"),

    # Domain 6: Editing & Refactoring (45..54)
    (45, "formatting", "textDocument/formatting", 6, "request"),
    (46, "range_formatting", "textDocument/rangeFormatting", 6, "request"),
    (47, "on_type_formatting", "textDocument/onTypeFormatting", 6, "request"),
    (48, "prepare_rename", "textDocument/prepareRename", 6, "request"),
    (49, "rename", "textDocument/rename", 6, "request"),
    (50, "linked_editing_range", "textDocument/linkedEditingRange", 6, "request"),
    (51, "code_action", "textDocument/codeAction", 6, "request"),
    (52, "code_action_resolve", "codeAction/resolve", 6, "request"),
    (53, "execute_command", "workspace/executeCommand", 6, "request"),
    (54, "apply_edit", "workspace/applyEdit", 6, "server_rpc"),

    # Domain 7: Presentation & Tokens (55..60)
    (55, "semantic_tokens_full", "textDocument/semanticTokens/full", 7, "request"),
    (56, "semantic_tokens_delta", "textDocument/semanticTokens/full/delta", 7, "request"),
    (57, "semantic_tokens_range", "textDocument/semanticTokens/range", 7, "request"),
    (58, "folding_range", "textDocument/foldingRange", 7, "request"),
    (59, "code_lens", "textDocument/codeLens", 7, "request"),
    (60, "code_lens_resolve", "codeLens/resolve", 7, "request"),

    # Domain 8: Diagnostics (61..63)
    (61, "document_diagnostic", "textDocument/diagnostic", 8, "request"),
    (62, "workspace_diagnostic", "workspace/diagnostic", 8, "request"),
    (63, "publish_diagnostics", "textDocument/publishDiagnostics", 8, "server_rpc"),

    # Domain 9: Workspace & File Operations (64..75)
    (64, "workspace_folders", "workspace/workspaceFolders", 9, "server_rpc"),
    (65, "did_change_workspace_folders", "workspace/didChangeWorkspaceFolders", 9, "notification"),
    (66, "did_change_configuration", "workspace/didChangeConfiguration", 9, "notification"),
    (67, "configuration", "workspace/configuration", 9, "server_rpc"),
    (68, "did_change_watched_files", "workspace/didChangeWatchedFiles", 9, "notification"),
    (69, "did_create_files", "workspace/didCreateFiles", 9, "notification"),
    (70, "will_create_files", "workspace/willCreateFiles", 9, "request"),
    (71, "did_rename_files", "workspace/didRenameFiles", 9, "notification"),
    (72, "will_rename_files", "workspace/willRenameFiles", 9, "request"),
    (73, "did_delete_files", "workspace/didDeleteFiles", 9, "notification"),
    (74, "will_delete_files", "workspace/willDeleteFiles", 9, "request"),
    (75, "text_document_content", "textDocument/content", 9, "request"),

    # Domain 10: Memory Mutation Scenarios (76..78)
    (76, "mutation_case_a", "mutation/caseA", 10, "mutation"),
    (77, "mutation_case_b", "mutation/caseB", 10, "mutation"),
    (78, "mutation_case_c", "mutation/caseC", 10, "mutation"),
]


def build_params(key, sc, repo_dir, is_ra=False):
    """Constructs strictly valid LSP 3.17 params conforming to spec schemas."""
    f = sc.get("file") or (sc.get("location") and sc["location"][0])
    uri = f"file://{f}" if f and not str(f).startswith("file://") else (f or f"file://{repo_dir}")
    line = max(0, sc.get("location")[1] - 1) if sc.get("location") and len(sc["location"]) > 1 else 0
    col = max(0, sc.get("location")[2] - 1) if sc.get("location") and len(sc["location"]) > 2 else 0
    pos = {"line": line, "character": col}
    rng = {"start": pos, "end": pos}

    if key in ("definition", "declaration", "type_definition", "implementation", "references", "hover",
               "document_highlight", "prepare_call_hierarchy", "prepare_type_hierarchy",
               "prepare_rename", "linked_editing_range", "moniker"):
        return {
            "textDocument": {"uri": uri}, "position": pos,
            **({"context": {"includeDeclaration": True}} if key == "references" else {})
        }
    elif key in ("completion", "signature_help", "on_type_formatting"):
        return {
            "textDocument": {"uri": uri}, "position": pos,
            **({"ch": ";", "options": {"tabSize": 4, "insertSpaces": True}} if key == "on_type_formatting" else {})
        }
    elif key in ("document_symbol", "folding_range", "code_lens", "document_diagnostic",
                 "document_link", "document_color", "text_document_content"):
        return {"textDocument": {"uri": uri}}
    elif key == "workspace_symbol":
        return {"query": sc.get("query", "test")}
    elif key == "workspace_symbol_resolve":
        return sc.get("item", {"name": "test_symbol", "kind": 12, "location": {"uri": uri, "range": rng}})
    elif key == "completion_resolve":
        return sc.get("item", {"label": sc.get("target", "item"), "kind": 3, "detail": "fn()"})
    elif key == "inlay_hint_resolve":
        return sc.get("item", {"position": pos, "label": ": i32", "kind": 1})
    elif key in ("code_lens_resolve", "code_action_resolve", "document_link_resolve", "moniker_resolve"):
        return sc.get("item", {"title": "resolve", "range": rng, "command": {"title": "resolve", "command": "noop"}})
    elif key in ("incoming_calls", "outgoing_calls"):
        return {"item": {"name": sc.get("target", "fn_call"), "kind": 12, "uri": uri, "range": rng, "selectionRange": rng}}
    elif key in ("supertypes", "subtypes"):
        return {"item": {"name": sc.get("target", "TraitType"), "kind": 5, "uri": uri, "range": rng, "selectionRange": rng}}
    elif key == "inline_value":
        return {"textDocument": {"uri": uri}, "range": rng, "context": {"frameId": 1, "stoppedLocation": rng}}
    elif key == "inline_completion":
        return {"textDocument": {"uri": uri}, "position": pos, "context": {"triggerKind": 1}}
    elif key == "color_presentation":
        return {"textDocument": {"uri": uri}, "color": {"red": 0.5, "green": 0.5, "blue": 0.5, "alpha": 1.0}, "range": rng}
    elif key == "semantic_tokens_full":
        return {"textDocument": {"uri": uri}}
    elif key == "semantic_tokens_delta":
        return {"textDocument": {"uri": uri}, "previousResultId": sc.get("prev_id", "0")}
    elif key == "semantic_tokens_range":
        return {"textDocument": {"uri": uri}, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 0}}}
    elif key == "formatting":
        return {"textDocument": {"uri": uri}, "options": {"tabSize": 4, "insertSpaces": True}}
    elif key == "range_formatting":
        return {"textDocument": {"uri": uri}, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 0}}, "options": {"tabSize": 4, "insertSpaces": True}}
    elif key == "rename":
        return {"textDocument": {"uri": uri}, "position": pos, "newName": sc.get("new_name", "renamed_sym")}
    elif key == "code_action":
        return {"textDocument": {"uri": uri}, "range": rng, "context": {"diagnostics": []}}
    elif key == "inlay_hint":
        return {"textDocument": {"uri": uri}, "range": {"start": {"line": 0, "character": 0}, "end": {"line": 2, "character": 0}}}
    elif key == "selection_range":
        return {"textDocument": {"uri": uri}, "positions": [pos]}
    elif key == "execute_command":
        if is_ra:
            return {"command": "rust-analyzer.reloadWorkspace", "arguments": []}
        return {"command": "ridx.resolveWorkspace", "arguments": []}
    elif key == "workspace_diagnostic":
        return {"identifier": "workspace"}
    elif key == "cancel_request":
        # String id guaranteed never to collide with numeric client next_id (int >= 1)
        return {"id": sc.get("req_id", "bench-cancel-0")}
    elif key == "set_trace":
        return {"value": "off"}
    elif key == "progress_cancel":
        return {"token": "token_1"}
    elif key == "did_open":
        return {"textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": sc.get("content", "pub fn dummy() {}")}}
    elif key in ("did_change_full", "did_change_inc"):
        return {"textDocument": {"uri": uri, "version": 2}, "contentChanges": [{"text": sc.get("content", "// change\n")}]}
    elif key in ("will_save", "will_save_wait"):
        return {"textDocument": {"uri": uri}, "reason": 1}
    elif key in ("did_save", "did_close"):
        return {"textDocument": {"uri": uri}}
    elif key == "did_change_watched_files":
        return {"changes": [{"uri": uri, "type": 2}]}
    elif key in ("did_create_files", "will_create_files"):
        return {"files": [{"uri": uri}]}
    elif key in ("did_rename_files", "will_rename_files"):
        return {"files": [{"oldUri": uri, "newUri": f"{uri}.renamed"}]}
    elif key in ("did_delete_files", "will_delete_files"):
        return {"files": [{"uri": uri}]}
    elif key == "did_change_workspace_folders":
        return {"event": {"added": [{"uri": f"file://{repo_dir}", "name": "ws"}], "removed": []}}
    elif key == "did_change_configuration":
        return {"settings": {}}
    return {}

# rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Live drop-in editor session verifier for `ridx` configured as `rust-analyzer.server.path`.

Verifies that:
1. Editor config files (`vscode-settings.json`, `zed-settings.json`, `neovim-lspconfig.lua`)
   configure `ridx` as a direct drop-in replacement for `rust-analyzer`.
2. `<server.path> --version` and `<server.path> --print-config-schema` succeed.
3. Launching `<server.path> --log-file <file> --no-log-buffering -v` (with NO `lsp` subcommand)
   starts the stdio JSON-RPC server on a real repository (`serde`) and serves `didOpen`,
   `definition`, `hover`, `references`, `didChange`, `didSave`, `shutdown`, and `exit`.
"""

import argparse
import json
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from lsp_process import SimpleLspProcess  # noqa: E402
from position_ab_parity_norm import normalize_answer  # noqa: E402


def verify_editor_configs(editors_dir):
    vscode = json.loads((editors_dir / "vscode-settings.json").read_text(encoding="utf-8"))
    zed = json.loads((editors_dir / "zed-settings.json").read_text(encoding="utf-8"))
    nvim = (editors_dir / "neovim-lspconfig.lua").read_text(encoding="utf-8")
    assert vscode.get("rust-analyzer.server.path") == "ridx", "vscode config must set rust-analyzer.server.path = ridx"
    assert zed["lsp"]["rust-analyzer"]["binary"]["path"] == "ridx", "zed config must set binary.path = ridx"
    assert 'cmd = { "ridx" }' in nvim, "neovim config must set cmd = { 'ridx' }"
    return {
        "vscode_server_path": vscode["rust-analyzer.server.path"],
        "zed_binary_path": zed["lsp"]["rust-analyzer"]["binary"]["path"],
        "neovim_cmd": ["ridx"],
    }


def run_dropin_session(ridx_bin, repo_dir, editors_dir):
    ridx_bin = Path(ridx_bin).resolve()
    repo_dir = Path(repo_dir).resolve()
    cfg_check = verify_editor_configs(Path(editors_dir))

    ver_proc = subprocess.run([str(ridx_bin), "--version"], capture_output=True, text=True, check=True)
    schema_proc = subprocess.run([str(ridx_bin), "--print-config-schema"], capture_output=True, text=True, check=True)
    schema_json = json.loads(schema_proc.stdout.strip())
    assert isinstance(schema_json, dict), "--print-config-schema must emit a JSON object"

    with tempfile.TemporaryDirectory(prefix="ridx_dropin_") as tmpdir:
        log_file = Path(tmpdir) / "ra_dropin.log"
        cmd = [str(ridx_bin), "--log-file", str(log_file), "--no-log-buffering", "-v"]
        srv = SimpleLspProcess(cmd, str(repo_dir), is_ra=False)
        steps = []
        try:
            assert log_file.exists(), "--log-file must create the log file on startup"
            rel_file = "serde_core/src/de/mod.rs"
            abs_file = repo_dir / rel_file
            uri = abs_file.as_uri()
            text = abs_file.read_text(encoding="utf-8")

            # Find line and col of `pub trait Deserialize<'de>: Sized`
            lines = text.splitlines()
            target_line = next(i for i, ln in enumerate(lines) if "pub trait Deserialize<" in ln)
            target_col = lines[target_line].index("Deserialize")

            srv.notify("textDocument/didOpen", {
                "textDocument": {"uri": uri, "languageId": "rust", "version": 1, "text": text},
            })
            steps.append({"step": "textDocument/didOpen", "file": rel_file, "line_1based": target_line + 1})

            pos_params = {
                "textDocument": {"uri": uri},
                "position": {"line": target_line, "character": target_col},
            }
            def_res, def_us, def_err = srv.request("textDocument/definition", pos_params, timeout=2.0)
            assert def_err is None and def_res, f"definition failed: {def_err}"
            def_norm = normalize_answer("definition", def_res, str(repo_dir))
            steps.append({
                "step": "textDocument/definition", "symbol": "Deserialize",
                "us": round(def_us, 1), "targets": def_norm["targets"],
            })

            hov_res, hov_us, hov_err = srv.request("textDocument/hover", pos_params, timeout=2.0)
            assert hov_err is None and hov_res, f"hover failed: {hov_err}"
            hov_norm = normalize_answer("hover", hov_res, str(repo_dir))
            hov_text = hov_norm["text"]
            assert "trait Deserialize<'de>: Sized" in hov_norm["sig_line"], f"expected full trait sig, got {hov_norm['sig_line']}"
            assert "**Provenance:**" in hov_text and "**Blast Radius:**" in hov_text, "expected Provenance and Blast Radius in hover"
            assert "data structure" in hov_text and "Serde" in hov_text, "expected doc comments in hover"
            steps.append({
                "step": "textDocument/hover", "symbol": "Deserialize",
                "us": round(hov_us, 1), "sig_line": hov_norm["sig_line"], "decl": hov_norm["decl"],
                "has_docs": True, "has_provenance": True, "has_blast_radius": True,
                "hover_markdown": hov_text,
            })

            ref_params = {**pos_params, "context": {"includeDeclaration": True}}
            ref_res, ref_us, ref_err = srv.request("textDocument/references", ref_params, timeout=2.0)
            assert ref_err is None and ref_res, f"references failed: {ref_err}"
            ref_norm = normalize_answer("references", ref_res, str(repo_dir))
            steps.append({
                "step": "textDocument/references", "symbol": "Deserialize",
                "us": round(ref_us, 1), "spans_count": len(ref_norm["spans"]), "ws_lines_count": len(ref_norm["ws_lines"]),
            })

            # Live edit (`didChange`) adding a comment line at top of buffer, then query shifted line
            edited_text = "// live drop-in edit\n" + text
            srv.notify("textDocument/didChange", {
                "textDocument": {"uri": uri, "version": 2},
                "contentChanges": [{"text": edited_text}],
            })
            shifted_params = {
                "textDocument": {"uri": uri},
                "position": {"line": target_line + 1, "character": target_col},
            }
            hov2_res, hov2_us, hov2_err = srv.request("textDocument/hover", shifted_params, timeout=2.0)
            assert hov2_err is None and hov2_res, f"hover after didChange failed: {hov2_err}"
            hov2_norm = normalize_answer("hover", hov2_res, str(repo_dir))
            steps.append({
                "step": "textDocument/hover (dirty buffer +1 line)", "symbol": "Deserialize",
                "us": round(hov2_us, 1), "sig_line": hov2_norm["sig_line"],
            })

            srv.notify("textDocument/didSave", {"textDocument": {"uri": uri}})
            steps.append({"step": "textDocument/didSave", "file": rel_file})
        finally:
            srv.close()

        log_contents = log_file.read_text(encoding="utf-8")
        assert "logger initialized" in log_contents, "log file must contain logger initialized line"
        assert "initialized LSP workspace" in log_contents, "log file must contain workspace init line"
        assert "request textDocument/hover" in log_contents, "log file must contain verbose hover request line"

    return {
        "configs": cfg_check,
        "version_stdout": ver_proc.stdout.strip(),
        "config_schema_stdout": schema_proc.stdout.strip(),
        "spawn_command": ["ridx", "--log-file", "<tmp>/ra_dropin.log", "--no-log-buffering", "-v"],
        "log_lines": log_contents.strip().splitlines(),
        "repo": repo_dir.name,
        "steps": steps,
    }


def main():
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--ridx-bin", required=True, type=Path)
    ap.add_argument("--repo", required=True, type=Path)
    ap.add_argument("--editors-dir", default=Path(__file__).resolve().parent.parent / "docs" / "editors", type=Path)
    ap.add_argument("--out", type=Path, default=None)
    args = ap.parse_args()
    res = run_dropin_session(args.ridx_bin, args.repo, args.editors_dir)
    out_text = json.dumps(res, indent=2) + "\n"
    if args.out:
        args.out.write_text(out_text, encoding="utf-8")
    print(out_text, end="")


if __name__ == "__main__":
    main()

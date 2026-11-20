# rust-intelligence — README translations
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0

"""Writes readme/README.<locale>.md: README.md's fenced blocks, badges and links around the prose in tr_<locale>.py.

Usage: python3 tools/readme_i18n/gen.py, then python3 tools/check_readme_translations.py
"""
import importlib.util
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(sys.argv[1]) if len(sys.argv) > 1 else Path(__file__).resolve().parents[2]
HERE = Path(__file__).parent
FENCES = re.findall(r"^```[^\n]*\n.*?^```[ \t]*$", (REPO / "README.md").read_text(), re.MULTILINE | re.DOTALL)
assert len(FENCES) == 5, len(FENCES)
SHA = subprocess.check_output(["git", "-C", str(REPO), "log", "-1", "--format=%H", "--", "README.md"], text=True).strip()
BASE = "https://github.com/alex09x/rust-intelligence/blob/main/"
ARCH_IMG = ('<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check '
            'runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>')
LANGS = [("en", "English", "README.md"), ("ru", "Русский", "readme/README.ru.md"), ("zh-CN", "简体中文", "readme/README.zh-CN.md"),
         ("es", "Español", "readme/README.es.md"), ("pt-BR", "Português (Brasil)", "readme/README.pt-BR.md"),
         ("ja", "日本語", "readme/README.ja.md"), ("de", "Deutsch", "readme/README.de.md"), ("fr", "Français", "readme/README.fr.md"),
         ("ko", "한국어", "readme/README.ko.md"), ("ar", "العربية", "readme/README.ar.md")]


def selector(cur):
    items = [f'  <b><a href="{BASE}{path}">{name}</a></b>' if code == cur else f'  <a href="{BASE}{path}">{name}</a>' for code, name, path in LANGS]
    return '<p align="center">\n' + " ·\n".join(items) + "\n</p>"


def render(code, t):
    rtl = code == "ar"
    o, c = ('<div dir="rtl">\n\n', "\n\n</div>") if rtl else ("", "")
    fence = (lambda i: f'<div dir="ltr">\n\n{FENCES[i]}\n\n</div>') if rtl else (lambda i: FENCES[i])
    src = f"https://github.com/alex09x/rust-intelligence/blob/{SHA}/README.md"
    return f"""<!-- translation-source: {SHA} -->

> [!NOTE]
> {t['note'].format(src=src, short=SHA[:7], main=BASE + 'README.md')}

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>{t['tagline']}</b><br>
  {t['subtitle']}
</p>

<p align="center">
  <a href="https://rust-intelligence.com"><img src="https://img.shields.io/badge/site-rust--intelligence.com-f74c00" alt="Website"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-c9ced4" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-1b1d20" alt="Platforms: Linux, macOS">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
</p>

{selector(code)}

{o}{t['intro']}

{t['platforms']}

---

## {t['h_why']}

- {t['why1']}
- {t['why2']}
- {t['why3']}

## {t['h_measured']}

{t['measured']}

| {t['th'][0]} | {t['th'][1]} | {t['th'][2]} | {t['th'][3]} |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- {t['run']}
- {t['not_counted']}
- {t['ready']}
- {t['memory']}
- {t['errors']}

{t['agreement']}

---

## {t['h_install']}

{t['install_platforms']}{c}

{fence(0)}

{o}{t['install_explain']}

{t['install_source']}{c}

{fence(1)}

{o}{t['install_then']}

{t['install_path']}

## {t['h_editors']}

{t['editors']}

- {t['zed']}
- {t['vscode']}
- {t['nvim']}
- {t['helix']}

## {t['h_shell']}{c}

{fence(2)}

{o}---

## {t['h_lsp']}

| {t['lsp_th'][0]} | {t['lsp_th'][1]} |
|---|---|
| {t['lsp'][0]} | `definition`, `declaration`, `implementation`, `typeDefinition` |
| {t['lsp'][1]} | `references`, `documentHighlight` |
| {t['lsp'][2]} | `hover`, `completion` ({t['auto_import']}), `signatureHelp`, `inlayHint`, `semanticTokens` |
| {t['lsp'][3]} | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| {t['lsp'][4]} | `prepareRename`, `rename`, `codeAction` ({t['assists']}), `executeCommand` |
| {t['lsp'][5]} | `formatting`, `rangeFormatting` |
| {t['lsp'][6]} | `documentSymbol`, `workspace/symbol` |
| {t['lsp'][7]} | {t['diag']} |
| {t['lsp'][8]} | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, {t['multi_root']} |

## {t['h_how']}{c}

{ARCH_IMG}

{o}{t['how']}

## {t['h_limits']}

- {t['lim1']}
- {t['lim2']}
- {t['lim3']}
- {t['lim4']}
- {t['lim6']}

---

## {t['h_dev']}

{t['dev']}{c}

{fence(3)}

{o}## {t['h_cite']}

{t['cite']}{c}

{fence(4)}

{o}## {t['h_license']}

{t['license_intro']}

- {t['apache']}
- {t['mit']}

{t['option']}{c}
"""


for code, _, path in LANGS[1:]:
    if not (HERE / f"tr_{code.replace('-', '_')}.py").exists():
        continue
    spec = importlib.util.spec_from_file_location(code, HERE / f"tr_{code.replace('-', '_')}.py")
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    (REPO / path).write_text(render(code, mod.T), encoding="utf-8")
    print("wrote", path)

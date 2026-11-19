<!-- translation-source: 46dfbadec54a2f01ab5d8cb2b05bfbed4c7f2540 -->

> [!NOTE]
> Dies ist eine Übersetzung der englischen README im Commit [46dfbad](https://github.com/alex09x/rust-intelligence/blob/46dfbadec54a2f01ab5d8cb2b05bfbed4c7f2540/README.md). Maßgeblich und aktuell ist die [englische README](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

<p align="center">
  <img src="../site/assets/logo-minimal.svg" width="96" height="96" alt="ridx logo">
</p>

<h1 align="center">Rust Intelligence (<code>ridx</code>)</h1>

<p align="center">
  <b>Rust-Code-Intelligenz mit einem Zehntel des Speichers.</b><br>
  Ein Language Server für Rust, der mit dem antwortet, was rustc bereits berechnet hat.
</p>

<p align="center">
  <a href="https://rust-intelligence.com"><img src="https://img.shields.io/badge/site-rust--intelligence.com-f74c00" alt="Website"></a>
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-c9ced4" alt="License: MIT OR Apache-2.0">
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS-1b1d20" alt="Platforms: Linux, macOS">
  <img src="https://img.shields.io/badge/LSP-3.18-1b1d20" alt="LSP 3.18">
  <a href="https://doi.org/10.5281/zenodo.24379816"><img src="https://zenodo.org/badge/DOI/10.5281/zenodo.24379816.svg" alt="DOI: 10.5281/zenodo.24379816"></a>
</p>

<p align="center">
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/README.md">English</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ru.md">Русский</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.zh-CN.md">简体中文</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.es.md">Español</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.pt-BR.md">Português (Brasil)</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ja.md">日本語</a> ·
  <b><a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.de.md">Deutsch</a></b> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.fr.md">Français</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ko.md">한국어</a> ·
  <a href="https://github.com/alex09x/rust-intelligence/blob/main/readme/README.ar.md">العربية</a>
</p>

`ridx` schreibt Definitionen, Referenzen und Typen einmal, während `cargo check`, in einen binären Index und liest
diesen Index direkt von der Festplatte. Ihr Editor bekommt Navigation, Hover, Vervollständigung, Referenzen und
Umbenennen, ohne einen zweiten Compiler im Speicher zu halten.

Linux und macOS. Nicht mit dem Rust Project oder der Rust Foundation verbunden.

---

## Warum

- **rust-analyzer hält seine eigene Analyse des gesamten Workspaces im Speicher**: 1.5 GB bei serde und
  11.3 GB bei Bevy in der Messung unten. Er wiederholt Namensauflösung, Makroexpansion und Typinferenz, die
  rustc bei jedem `cargo check` ohnehin ausführt.
- **Das meiste Analysierte sind Abhängigkeiten und std**, Code, den niemand bearbeitet. `ridx` indexiert jede
  Abhängigkeit einmal und teilt sie zwischen allen Projekten und Worktrees auf dem Rechner.
- **Was Sie gerade tippen, funktioniert auch**: Ungespeicherte Änderungen beantwortet eine Schicht im Speicher
  über dem Index, bis das Projekt wieder baut.

## Gemessen

Dieselbe Editor-Sitzung gegen beide Server: je Repository 40 Dateien nacheinander geöffnet, mit Dokumentsymbolen,
semantischen Tokens, Faltung und Inlay-Hinweisen, dann Hover, Definition, Typdefinition, Hervorhebung, Referenzen
und Vervollständigung an Stichprobenpositionen sowie eine Bearbeitung. Der Speicher umfasst Kindprozesse.

| Repository | Spitzenspeicher, ridx / rust-analyzer | CPU der Sitzung | Start bis bereit |
|---|---:|---:|---:|
| serde | 72 MB / 1.5 GB | 2.34 s / 6.97 s | 0.06 s / 3.27 s |
| tokio | 221 MB / 3.2 GB | 1.90 s / 35.9 s | 0.24 s / 4.77 s |
| rust-analyzer | 434 MB / 4.6 GB | 4.44 s / 66.7 s | 0.64 s / 5.22 s |
| bevy | 917 MB / 11.3 GB | 5.38 s / 170.5 s | 1.26 s / 15.5 s |

- ridx `f7105fcc`, 2026-11-14; ein Lauf (n = 1) auf Linux 6.8, AMD Ryzen 9 7950X, 64 GB. Rohdaten:
  [`bench/published/resources_f7105fcc.jsonl`](../bench/published/resources_f7105fcc.jsonl), Skript:
  [`bench/measure_resources.py`](../bench/measure_resources.py); die Latenz je Methode steht in den Rohdaten und
  auf der Website.
- **Nicht mitgezählt**: ridx antwortet aus einem vorher gebauten Index (`ridx index`, ein `cargo check` mit dem
  ridx-Treiber), der beim ersten Mal auf einem großen Workspace Minuten dauert. rust-analyzer analysiert den
  Workspace bei jedem Start, und diese Zeit steckt in seinen Zahlen.
- **„Bereit“** ist für ridx die Antwort auf `initialize` (Caches wärmen danach im Hintergrund weiter auf) und für
  rust-analyzer das Ende seiner Analyse plus 0.5 s ohne Nachrichten.
- **Speicher**: Die Zahl von ridx enthält von der Festplatte gemappte Indexseiten, die der Kernel unter Druck
  verwerfen und neu lesen kann. Die von rust-analyzer ist sein eigener Heap.
- rust-analyzer antwortete auf 26–31 Anfragen je Repository mit einem Fehler, ridx auf keine; die Fehlerarten wurden
  nicht erfasst. Fehlgeschlagene Anfragen sind in den Latenzzahlen enthalten.

Die Übereinstimmung der Antworten mit rust-analyzer je LSP-Methode wird mit der Suite in [`bench/`](../bench/)
gemessen und je Release in [`CHANGELOG.md`](../CHANGELOG.md) festgehalten. Die vollständige Matrix mit 78 Methoden für `v0.2.0` steht in
[`docs/BENCHMARK_MATRIX.md`](../docs/BENCHMARK_MATRIX.md) und
[`LSP_BENCHMARK_COMPARISON_REPORT.md`](../LSP_BENCHMARK_COMPARISON_REPORT.md).

---

## Installation

Linux (x86_64, arm64) und macOS auf Apple Silicon, mit [rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

Der Installer fügt Rust 1.98.1 hinzu, die Toolchain, in der `ridx-driver` läuft (er linkt gegen die Bibliotheken
dieses Compilers; Ihre Projekte behalten jede Toolchain, ridx baut den Index mit der richtigen), lädt das
Release-Archiv für Ihre Plattform von GitHub, prüft dessen SHA-256 und legt `ridx` und `ridx-driver` nach
`~/.local/bin` (`RIDX_INSTALL_DIR` ändert das, `RIDX_VERSION=v0.3.7` legt ein Release fest).

Stattdessen aus dem Quellcode bauen:

```bash
rustup toolchain install 1.98.1 --component rustc-dev,llvm-tools-preview,rust-src,rustfmt,clippy
git clone https://github.com/alex09x/rust-intelligence
cd rust-intelligence
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx --locked
RUSTC_BOOTSTRAP=1 cargo +1.98.1 install --path crates/ridx-driver --locked
```

Öffnen Sie dann einen Cargo-Workspace im Editor. Gibt es noch keinen Index, baut `ridx` ihn bei der ersten
Verwendung und meldet den Fortschritt an den Editor; nach dem Build lädt er den Index neu und indexiert nach dem
Speichern im Hintergrund neu. `ridx index .` baut ihn in der Shell (`--all-features`, `--target <triple>` wählen
den cfg).

Findet ein Desktop-Editor `ridx` nicht, geben Sie ihm den vollen Pfad aus `which ridx`.

## Editoren

`ridx` ohne Unterbefehl verhält sich auf der Kommandozeile wie `rust-analyzer` (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), daher kann ein für rust-analyzer eingerichteter Editor stattdessen ridx starten.

- **Zed**: Die Erweiterung [`editors/zed`](../editors/zed) registriert `ridx lsp` als Language Server
  `rust-intelligence` (Befehlspalette: `zed: install dev extension`), und
  [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) wählt ihn für Rust aus.
- **VS Code**: [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) richtet die
  rust-analyzer-Erweiterung auf `ridx` (`"rust-analyzer.server.path": "ridx"`).
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  die Wurzel wird zuerst über `Cargo.lock` gesucht, sodass ein Workspace-Mitglied den Index des Workspaces öffnet).
- **Helix**: ein `[language-server.ridx]` mit `command = "ridx"`, `args = ["lsp"]` in `languages.toml`.

## Aus der Shell

```bash
ridx index .                     # build or refresh the index
ridx def . my_crate::run         # go to definition, by path or file:line:col
ridx refs . src/main.rs:12:15    # references across the workspace and its dependencies
ridx hover . src/main.rs:12:15
ridx type-at . src/main.rs:12:15
ridx symbols . Config
ridx inspect .                   # shards, crates and record counts
ridx compare -c all . my_crate::run   # side by side with rust-analyzer
```

---

## LSP-Methoden

| Kategorie | Methoden |
|---|---|
| Navigation | `definition`, `declaration`, `implementation`, `typeDefinition` |
| Referenzen | `references`, `documentHighlight` |
| Intelligenz | `hover`, `completion` (mit Auto-Import), `signatureHelp`, `inlayHint`, `semanticTokens` |
| Hierarchien | `prepareCallHierarchy`, `incomingCalls`, `outgoingCalls`, `prepareTypeHierarchy`, `supertypes`, `subtypes` |
| Refactoring | `prepareRename`, `rename`, `codeAction` (per Kompilierung geprüfte Assists), `executeCommand` |
| Formatierung | `formatting`, `rangeFormatting` |
| Symbole | `documentSymbol`, `workspace/symbol` |
| Diagnosen | `publishDiagnostics` aus einem `cargo check` im Hintergrund beim Speichern, Pull-Diagnosen |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, Multi-Root |

## Wie es funktioniert

```
 editor ──JSON-RPC (stdio)──► ridx lsp
                               ├─ OverlayManager (RAM): open buffers, live edits
                               ├─ WorkspaceIndex (mmap): shards of this workspace
                               └─ flycheck thread: cargo check on save
                                        │
              ┌─────────────────────────┴─────────────────────────┐
   ~/.cache/ridx/v5/objects/                          target/ridx/variants/<id>/
   dependencies, once per source hash and target      the workspace's own crates
   shared by every project                            + a manifest scoped to Cargo.lock
                                        ▲
                         ridx-driver (rustc wrapper inside cargo check)
```

Der `.ridx`-v5-Shard ist Little-Endian mit 8-Byte-ausgerichteten Datensätzen fester Größe: ein Header, ein
String-Pool, die Dateitabelle (16 B pro Eintrag), Definitionen (40 B) mit einem FNV-1a-Hashindex, Owner- und
Deref-Tabellen für Empfängerketten, nach Position sortierte Referenzen (48 B) mit einem Ziel-Hashindex sowie
Tabellen für Typen, Impls, Felder und Generics. Suchen sind binäre Suchen im gemappten Speicher; nichts wird in den
Heap geparst, und die Shard-Dateien werden nach dem Mappen geschlossen.

## Bekannte Einschränkungen

- **Antworten stammen aus dem letzten Build.** Seitdem getippter Code wird aus dem offenen Puffer beantwortet und
  holt auf, sobald das Projekt wieder baut; ein Crate, der nicht baut, behält seinen letzten guten Index.
- **Ein cfg.** Wie `cargo check` sieht der Index ein Target und einen Feature-Satz; Code hinter anderen
  `#[cfg]`s oder Features wird nicht indexiert. Auswahl mit `ridx index --all-features` / `--target`.
- **Windows** wird noch nicht unterstützt.
- **Standardbibliothek**: Navigation und Hover nach `std`, `core` und `alloc` funktionieren; Referenzen darauf
  werden nur im Workspace gesucht, und sie lassen sich nicht umbenennen.
- **Eingehende Aufrufe** listen einen Aufrufer dort, wo die Funktion aufgerufen wird (`name(` oder `name::<…>(`);
  rust-analyzer listet auch als Wert übergebene Funktionen, etwa ein an `add_systems` übergebenes System.
- `vec![]` zählt als Referenz auf `Vec`; rust-analyzer zählt es nicht.

---

## Entwicklung

Jede Quelldatei bleibt bei höchstens 400 Zeilen.

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Zitieren

Wenn Sie ridx in Forschung oder Dokumentation verwenden, zitieren Sie bitte das archivierte Release auf Zenodo:
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) für das Projekt (verweist auf die neueste Version), [10.5281/zenodo.24379817](https://doi.org/10.5281/zenodo.24379817) für genau 0.3.7.
[`CITATION.cff`](../CITATION.cff) enthält die Metadaten, die GitHubs „Cite this repository“ verwendet.

```bibtex
@software{panasenko_2026_24379817,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.7},
  doi       = {10.5281/zenodo.24379817},
  url       = {https://doi.org/10.5281/zenodo.24379817},
}
```

## Lizenz

Lizenziert unter einer der folgenden Lizenzen:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) oder http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) oder http://opensource.org/licenses/MIT)

nach Ihrer Wahl.

<!-- translation-source: a23435e39ef0f3e231c89e3860254dfdddb82f38 -->

> [!NOTE]
> Dies ist eine Übersetzung der englischen README im Commit [a23435e](https://github.com/alex09x/rust-intelligence/blob/a23435e39ef0f3e231c89e3860254dfdddb82f38/README.md). Maßgeblich und aktuell ist die [englische README](https://github.com/alex09x/rust-intelligence/blob/main/README.md).

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
  <img src="https://img.shields.io/badge/platform-Linux%20%7C%20macOS%20%7C%20Windows-1b1d20" alt="Platforms: Linux, macOS, Windows">
  <img src="https://img.shields.io/badge/protocol-LSP-1b1d20" alt="Language Server Protocol">
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

Linux, macOS und Windows. Nicht mit dem Rust Project oder der Rust Foundation verbunden.

---

## Warum

- **rust-analyzer hält seine eigene Analyse des gesamten Workspaces im Speicher**: 1.5 GB bei serde und
  11.3 GB bei Bevy in der Messung unten. Er wiederholt Namensauflösung, Makroexpansion und Typinferenz, die
  rustc bei jedem `cargo check` ohnehin ausführt.
- **Das meiste Analysierte sind Abhängigkeiten und std**, Code, den niemand bearbeitet. `ridx` indexiert jede
  Abhängigkeit einmal und teilt sie zwischen allen Projekten und Worktrees auf dem Rechner.
- **Was Sie tippen, funktioniert trotzdem**: Ungespeicherte Änderungen beantwortet sofort eine Schicht im
  Speicher über dem Index, und 250 ms nach der letzten Änderung werden die geänderten Crates aus den Puffern
  in einen Index der Sitzung kompiliert, sodass auch Referenzen und Umbenennen den neuen Code sehen. Auch die Fehler des Compilers darin werden gemeldet.

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

Linux (x86_64, arm64), macOS (Apple Silicon, Intel) und Windows (x86_64), mit
[rustup](https://rustup.rs):

```bash
curl -fsSL https://rust-intelligence.com/install.sh | sh
```

Unter Windows in der PowerShell:

```powershell
powershell -ExecutionPolicy Bypass -c "irm https://rust-intelligence.com/install.ps1 | iex"
```

Der Installer fügt Rust 1.98.1 hinzu, die Toolchain, in der `ridx-driver` läuft (er linkt gegen die Bibliotheken
dieses Compilers; Ihre Projekte behalten jede Toolchain, ridx baut den Index mit der richtigen), lädt das
Release-Archiv für Ihre Plattform von GitHub, prüft dessen SHA-256 und legt `ridx` und `ridx-driver` nach
`~/.local/bin` (unter Windows nach `%USERPROFILE%\.local\bin`, das er in Ihren PATH aufnimmt;
`RIDX_INSTALL_DIR` ändert das, `RIDX_VERSION=v0.3.14` legt ein Release fest).

Oder von [crates.io](https://crates.io/crates/ridx). Bauen Sie beide Programme mit Rust 1.98.1: `ridx-driver` linkt gegen
die Bibliotheken dieses Compilers und braucht deshalb `rustc-dev` und diese beiden Flags, und `ridx` führt den
Index-Build mit der Toolchain aus, mit der es gebaut wurde. Aus einem Checkout nehmen Sie `--path crates/ridx` und
`--path crates/ridx-driver` statt der Crate-Namen.

```bash
rustup toolchain install 1.98.1 --profile minimal --component rustc-dev,rust-src
cargo +1.98.1 install ridx
RUSTC_BOOTSTRAP=1 RUSTFLAGS="-C prefer-dynamic" cargo +1.98.1 install ridx-driver
```

Öffnen Sie dann einen Cargo-Workspace im Editor. Gibt es noch keinen Index, baut `ridx` ihn bei der ersten
Verwendung und meldet den Fortschritt an den Editor; nach dem Build lädt er den Index neu und indexiert nach dem
Speichern im Hintergrund neu. `ridx index .` baut ihn in der Shell (`--all-features`, `--target <triple>` wählen
den cfg).

Findet ein Desktop-Editor `ridx` nicht, geben Sie ihm den vollen Pfad aus `which ridx`.

## Editoren

`ridx` ohne Unterbefehl verhält sich auf der Kommandozeile wie `rust-analyzer` (`--version`, `--stdio`,
`--log-file`, `--print-config-schema`), daher kann ein für rust-analyzer eingerichteter Editor stattdessen ridx starten.

- **VS Code**: Die Erweiterung [Rust Intelligence](https://marketplace.visualstudio.com/items?itemName=alex09x.rust-intelligence)
  ([`editors/vscode`](../editors/vscode)) startet `ridx lsp` und bietet an, `ridx` zu installieren, wenn sie es nicht findet.
  Oder richten Sie die rust-analyzer-Erweiterung auf `ridx`:
  [`docs/editors/vscode-settings.json`](../docs/editors/vscode-settings.json) (`"rust-analyzer.server.path": "ridx"`).
- **Zed**: Die Erweiterung [`editors/zed`](../editors/zed) registriert `ridx lsp` als Language Server
  `rust-intelligence` und lädt `ridx` herunter, wenn es nicht im PATH liegt (Befehlspalette: `zed: install dev
  extension`); [`docs/editors/zed-settings.json`](../docs/editors/zed-settings.json) wählt ihn für Rust aus.
- **Neovim**: [`docs/editors/neovim-lspconfig.lua`](../docs/editors/neovim-lspconfig.lua) (`cmd = { "ridx" }`,
  die Wurzel wird zuerst über `Cargo.lock` gesucht, sodass ein Workspace-Mitglied den Index des Workspaces öffnet).
- **Helix**: [`docs/editors/helix-languages.toml`](../docs/editors/helix-languages.toml) deklariert den Language Server `ridx` und wählt ihn in `languages.toml` für Rust aus.

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
| Refactoring | `prepareRename`, `rename`, `codeAction` (kompiliersichere Assists), `executeCommand` |
| Formatierung | `formatting`, `rangeFormatting` |
| Symbole | `documentSymbol`, `workspace/symbol` |
| Diagnosen | `publishDiagnostics` aus einem `cargo check` im Hintergrund beim Speichern und aus dem Kompilieren ungespeicherter Puffer, Pull-Diagnosen |
| Workspaces | `didChangeWorkspaceFolders`, `didChangeWatchedFiles`, Multi-Root |

## Wie es funktioniert

<p align="center"><img src="../site/assets/architecture.svg" width="900" alt="How ridx works: at build time cargo check runs ridx-driver inside rustc and writes .ridx shards to disk; at edit time ridx lsp maps them and answers the editor"></p>

rustc löst ohnehin bei jedem `cargo check` jeden Namen auf, expandiert jedes Makro und inferiert jeden Typ.
`ridx` behält diese Antworten, statt sie neu zu berechnen.

- **Der Compiler schreibt den Index.** `ridx index` führt `cargo check` mit `ridx-driver` als rustc-Wrapper
  aus. Nach der Analyse durchläuft der Treiber das HIR und die Ergebnisse der Typprüfung von rustc und
  schreibt pro Crate einen `.ridx`-Shard: Definitionen, jede aufgelöste Referenz (Methodenaufrufe und Felder
  so, wie rustc sie über Auto-Deref und Trait-Auswahl aufgelöst hat), Typen, Impls, Felder, Generics, Makros
  und `use`-Einträge. Danach läuft die Kompilierung normal weiter, der Check selbst bleibt also unverändert.
- **Abhängigkeiten werden einmal pro Rechner indexiert.** Der Shard einer Abhängigkeit kommt in einen
  inhaltsadressierten Speicher, mit einem Schlüssel aus Crate, Version, Zielplattform, cfg-Flags und den
  Digests dessen, wogegen sie kompiliert wurde; liegt dort ein gültiger Shard, extrahiert der Treiber das
  Crate nicht erneut. Projekte und Worktrees teilen sich diese Shards, und `std`, `core` und `alloc` werden einmal pro
  Toolchain indexiert.
- **Lesen heißt abbilden, nicht parsen.** Der v5-Shard ist Little-Endian mit Datensätzen fester Größe in
  Abschnitten, die auf 8 Byte ausgerichtet sind: ein String-Pool, Dateien (16 B), Definitionen (40 B) mit FNV-1a-Hashindex, nach
  Position sortierte Referenzen (48 B) mit Hashindex der Ziele sowie Tabellen für Typen, Impls, Felder und
  Generics. Eine Abfrage ist eine Hash-Probe, eine binäre Suche oder ein Durchlauf über abgebildete Seiten. Seiten, nach
  denen niemand fragt, werden nie gelesen, und die übrigen kann der Kernel bei Speicherdruck verwerfen.
- **Neubauten sind inkrementell und sicher.** Nach dem Speichern indexiert der Server im Hintergrund neu;
  wenn sich nur Quellen des Workspace geändert haben, ruft er den Treiber direkt für die geänderten Crates
  auf, ohne Cargo. Ein Crate mit Kompilierfehlern wird aus allem indexiert, was aufgelöst wurde; eines, das
  der Compiler nicht so weit bringt (ein Syntaxfehler), behält seinen letzten guten Shard. Ein anderer
  Feature-Satz oder ein anderes Target bekommt eine eigene Variante neben der ersten.
- **Code, der zwischen Builds getippt wird,** wird zeilenweise auf den kompilierten Text abgebildet, und den
  geänderten Code analysiert ein kleines Frontend ohne Compiler ([`crates/ridx-syntax`](../crates/ridx-syntax):
  Lexer, Parser, Namensauflösung, Typinferenz), das alle anderen Typen und Elemente aus dem Index liest.
  250 ms nach der letzten Änderung werden die Crates der geänderten Puffer aus ihrem Text in einen Index der
  Sitzung kompiliert (in die Dateien wird nichts geschrieben); er beantwortet Referenzen, Umbenennen und
  Navigation bis zum nächsten Build.

Das ganze Design, mit dem Aufbau eines Shards und wo jeder Teil im Code liegt:
[`docs/ARCHITECTURE.md`](../docs/ARCHITECTURE.md).

## Bekannte Einschränkungen

- **Ungespeicherter Code erreicht den Index mit Verzögerung.** Die geänderten Crates werden 250 ms nach der
  letzten Änderung aus den Puffern kompiliert, was von Sekundenbruchteilen bis zu einigen Sekunden für ein
  großes Crate dauert; bis dahin kommen Referenzen und Navigation aus dem vorherigen Index, auf die Änderung
  abgebildet. Crates, die von einem geänderten abhängen, sehen die Änderung nach dem Speichern.
- **Ein cfg.** Wie `cargo check` sieht der Index ein Target und einen Feature-Satz; Code hinter anderen
  `#[cfg]`s oder Features wird nicht indexiert. Auswahl mit `ridx index --all-features` / `--target`.
- **Vorgefertigte Binaries** gibt es für Linux x86_64 und arm64, macOS auf Apple Silicon und Intel sowie
  Windows x86_64; auf anderen Plattformen installieren Sie von crates.io.
- **Standardbibliothek**: Navigation und Hover nach `std`, `core` und `alloc` funktionieren; Referenzen darauf
  werden nur im Workspace gesucht, und sie lassen sich nicht umbenennen.
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
[10.5281/zenodo.24379816](https://doi.org/10.5281/zenodo.24379816) für das Projekt (verweist auf die neueste Version), [10.5281/zenodo.24576729](https://doi.org/10.5281/zenodo.24576729) für genau 0.3.16.
[`CITATION.cff`](../CITATION.cff) enthält die Metadaten, die GitHubs „Cite this repository“ verwendet.

```bibtex
@software{panasenko_2026_24576729,
  author    = {Panasenko, Alexander},
  title     = {Rust Intelligence (ridx) — a Rust language server answering from a compiler-built on-disk index},
  month     = nov,
  year      = 2026,
  publisher = {Zenodo},
  version   = {0.3.16},
  doi       = {10.5281/zenodo.24576729},
  url       = {https://doi.org/10.5281/zenodo.24576729},
}
```

## Lizenz

Lizenziert unter einer der folgenden Lizenzen:

- Apache License, Version 2.0 ([LICENSE-APACHE](../LICENSE-APACHE) oder http://www.apache.org/licenses/LICENSE-2.0)
- MIT license ([LICENSE-MIT](../LICENSE-MIT) oder http://opensource.org/licenses/MIT)

nach Ihrer Wahl.

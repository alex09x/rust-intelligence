# Changelog

All notable changes to `rust-intelligence` (`ridx`) are documented in this file.

## [0.3.9] - 2026-11-20

Verified by the workspace test suite and by an LSP session replayed against the release build on a Linux node (references to code typed since the last save, `minVersion`, `ridx/status`). The per-method agreement suite was not re-run for this release. The index format is unchanged.

### Added
- **Answers on code typed since the last save**: once the edited buffers have been unchanged for 250 ms, the crates owning them are compiled again from the buffers' text (nothing is written to the files) into an index of the session under `target/ridx/sessions/`, so references, rename and navigation include the new code. It answers until a newer published index loads or no buffer is edited any more. `RIDX_SESSION_INDEX=0` turns it off.
- **`minVersion` and `ridx/status`** for tools driving the server: a request carrying `params.minVersion` waits until the index answering covers that version of the document (or fails with `-32803` after the timeout); `ridx/status` reports per workspace root whether an index is loaded, its generation, a build running and the last error, and for a document the version the index covers. The methods and options supported are listed in `capabilities.experimental.ridx`. `$/cancelRequest` ends a request still queued or waiting.
- **Integration contract**: [`docs/integration.md`](docs/integration.md) describes for tools using Rust Intelligence the lifecycle, broken code, unsaved text, diagnostics, code actions, the `ridx/*` methods and the index location, with what is available and what is planned.
- **Install from crates.io**: `cargo +1.98.1 install ridx` and `ridx-driver`, as the README describes.

### Fixed
- **Crates that do not compile** are indexed from everything that resolved: definitions, references, types and calls are answered in them, inside the bodies with errors too. 0.3.8 indexed only crates that compiled.
- **Compiler diagnostics on start**: every workspace of the session is checked after `initialized`, one after the other, not only after a save; a save during that check keeps the roots still to check.

### Known issues
- A browser-downloaded macOS archive still needs `xattr -dr com.apple.quarantine` before the first run (see 0.3.8).

## [0.3.8] - 2026-11-19

Verified by the workspace test suite and by the editor configurations in the README run against the release binary: Neovim (definition, hover, completion) and Helix (`hx --health rust` and an LSP session opened on a workspace member). The per-method agreement suite was not re-run for this release. The index format is unchanged.

### Added
- **Signed and notarized macOS binaries**: `ridx` and `ridx-driver` for Apple Silicon are signed with a Developer ID, run under the hardened runtime and are notarized by Apple. The release stops instead of shipping an unsigned build when signing or notarization fails. An archive downloaded with a browser needs `xattr -dr com.apple.quarantine` before the first run: Gatekeeper refuses a quarantined `ridx-driver`, which loads `librustc_driver` from the local rustup toolchain. The installer does not quarantine its download.
- **Helix**: [`docs/editors/helix-languages.toml`](docs/editors/helix-languages.toml) declares the `ridx` language server and selects it for Rust.
- **Documentation**: how the index works and an architecture diagram, in the README and on the site.

### Fixed
- **Workspace root**: an editor that opens a workspace member as the root (Helix uses the nearest `Cargo.toml`) now indexes and answers for the whole Cargo workspace; workspace folders added later and the current-directory fallback are lifted to their workspace the same way.

## [0.3.7] - 2026-11-19

The first public release. Verified by the workspace test suite, by Zed's LSP traffic replayed against the server, by hand in Zed, and by rust-analyzer's own go-to-definition fixtures and 26 cross-crate scenarios run against both servers (`bench/ra_fixtures.py`, `bench/fixtures/cross_crate_definition.txt`). The per-method agreement suite was not re-run for this release. The index format is unchanged.

### Added
- **Release binaries and an installer**: `curl -fsSL https://rust-intelligence.com/install.sh | sh` installs `ridx` and `ridx-driver` for Linux x86_64, Linux arm64 and macOS on Apple Silicon, with the Rust 1.98.1 toolchain the driver runs on. A binary built on another machine finds that toolchain under the local rustup home.
- **Indexing on first use**: a workspace opened without an index is indexed in the background, with progress shown in the editor; buffers opened before it finishes are answered as soon as it loads. Workspace folders added later are indexed the same way.
- **Definitions the index cannot give**, found in the syntax tree: keywords, labels and lifetimes, `mod foo;` (its file), macro calls (local `macro_rules!` in scope, `#[macro_use]`, qualified and unimported calls, the standard library before a dependency), `Self`, the items of an `impl Trait for`, a shorthand field (`Point { x }` gives the field and the binding), and a crate name, which opens the crate's root (a renamed or registry dependency, `std`).
- **Primitive types**: definition and hover of `u8`, `str`, `bool` and the rest open the standard library's documentation of the primitive.
- **A caret just past a name** (where an editor leaves it after a double click or typing) answers as on the name, for definition, hover, highlights, references and the other position requests.
- **The site** (https://rust-intelligence.com) with the measured memory, CPU and start-up against rust-analyzer, their raw results and script (`bench/measure_resources.py`, `bench/published/resources_f7105fcc.jsonl`).

### Fixed
- **Compiler diagnostics**: none for a file changed while the check ran; positions in the saved file's coordinates for Zed, which shifts them itself; a source changed outside the editor is checked again.
- **Background reindex**: reaches open buffers and the client (inlay hints, semantic tokens and code lenses refresh); an open buffer is rebased only onto a text every one of its units built; an unchanged index is not rebuilt, and each unit compiles once.
- **Signature help** stays silent on declarations: an item's own parameter list, `impl`, enum variants, bodies after `-> S`, comments and string, raw, byte and C-string literals.
- **Add explicit type** sees the imports of the block around the `let`; **Format Selections** indents a line to its block.

## [0.3.6] - 2026-11-15

Verified by the workspace test suite and by hand in Zed (hover, Cmd+click, references, rename, completion with auto-import, code actions, signature help, inlay hints, linked editing, document links, formatting, runnables) on edited, reverted and failed-to-build workspaces. The benchmark suite was not re-run for this release. The index format is unchanged; a re-index is recommended (source snapshots are written from this release on).

### Added
- **Zed**: an extension registering `ridx lsp`; `experimental/runnables` for Zed's run buttons; a background reindex on save and when sources change outside the editor.
- **Code actions**: `Add explicit type` (the whole type, default type arguments left out, offered only where every name in it is in scope); `Replace qualified path with use`; `Inline macro`; documentation templates on trait impl methods; many more `Inline into all callers`, `Implement default members` and `Convert to tuple/named struct` cases.
- **Completion**: name completions import what they offer by a path that compiles (public path, crate root, `std` re-exports, the module level typed at, a package's library from its binaries), labelled `(use path)` with `std` first; completion items show their declaration's doc comment.
- **Hover** on code typed since the last index.

### Fixed
- **Edited, reverted and failed builds**: a crate that fails to rebuild keeps its last good shards (by unit, dependencies included, with no window where a running server loses them); a file changed on disk before it is opened maps onto the text it was compiled from (source snapshots); lines between two edits, and parameter hints, follow the edit; an edited import blocks the index only for the names it may change; hover in an edited buffer answers as in a clean one; nothing is answered from a stale workspace file with no snapshot.
- **Local bindings**: closure parameters, `for`, `if let`/`while let` and match-arm patterns, destructuring `let`s and or-patterns bind their names in their own scope, so rename, references, highlight and linked editing no longer reach a shadowed name.
- **Hover**: nothing on whitespace, comments, block comments or keywords; a loop's or an async fn's binding shows its own type; a literal shows its type; never an answer read off a newer text of the file than the index compiled.
- **Navigation**: jumps into an open std or dependency file land; std locations point into the driver toolchain's sources; a crate appearing on reload is a workspace crate.
- **Formatting**: a document the editor has not opened is never formatted from the disk; `Format Selections` keeps indentation, the selection's end and the text of multi-line literals; typing `;` or `}` no longer reformats the whole file.
- **Other**: code actions in a dependency's source offer nothing; outgoing calls skip strings and comments; a compiler diagnostic carries its primary span's label; inlay hints leave out default type arguments; a panicking request no longer ends the LSP session.

## [0.3.5] - 2026-11-11

Figures: the suite on the `0.3.5` and `0.3.4` release builds on one index (the driver is unchanged; no re-index needed). Every `Convert to tuple struct` edit `0.3.5` offers on the 7 corpora was applied and compiled (`bench/code_action_compile.py`): `93` of `93` compile; rust-analyzer's compile in `105` of `141`.

### Changed
- **`Convert to tuple struct`** is offered on many more structs, always with an edit that compiles: on a struct with a `where` clause (moved after the fields); beside a method named like a field; on a struct with a third-party derive the index has seen on a tuple struct that compiled (its helper attributes pass when each key they use was seen there; clap's `Parser`/`Args` stay refused); on a struct built, destructured or implemented in other files (`Name { .. }` and `Self { .. }` there are rewritten); with struct updates from a plain path (`..self`) and rest patterns (`..`). Refused where rewriting could not be complete: a free type alias of the struct, a bare path handed to a macro that may build it, fields under `#[cfg]`. `codeAction` parity `1206` → `1236` of `2070`, `codeAction/resolve` `881` → `884` of `2074`; no row lost in any of the 38 methods. Latency p50 `1.15` → `1.26 ms`, p95 `15.3` → `16.1 ms`.

### Fixed
- **`Convert to tuple struct`**: a field access inside the struct's own literal (`Self { syntax: self.syntax.clone() }`) is rewritten with it; it was left as `self.syntax`, which breaks the build (E0615) when a method shares the field's name.

## [0.3.4] - 2026-11-11

Figures: the suite on the `0.3.4` and `0.3.3` release builds on one index (no re-index needed).

### Fixed
- **Type inference**: a projection on a type parameter whose bounds do not include the projection's trait goes through the impls, as rustc selects it: `<I as IntoIterator>::IntoIter` with `I: Iterator` is `I` by the blanket `impl<I: Iterator> IntoIterator for I`. A parameter bounded by the trait keeps the projection abstract. `typeDefinition` `1225` → `1226` of `1308`; no row lost in any of the 38 methods.

## [0.3.3] - 2026-11-11

Figures: the suite on the `0.3.3` and `0.3.2` release builds on one index (the driver is unchanged; no re-index needed).

### Fixed
- **`textDocument/typeDefinition`**: `Self` resolves through an impl only when one written in the source encloses the cursor. The nearest impl item before the line was taken, enclosing it or not, so `Box<Self>` in a field of a derived struct listed std's `MetadataExt` from the derive's impl. Parity `1224` → `1225` of `1308`, no row lost; median latency unchanged at `0.55 ms`.

## [0.3.2] - 2026-11-11

Figures: the suite on the `0.3.2` and `0.3.1` release builds, each on an index its own driver built afresh with the same Mitts build. Indexes built by `0.3.1` should be rebuilt (`ridx index .`): the driver now records the names of attribute and derive macros.

### Changed
- **`textDocument/references`**: an attribute or derive macro's name where it is written (`#[tokio::test]`, `#[derive(Deserialize)]`) is a reference to the macro (`tokio::test` from a test: `14` → `881` references, rust-analyzer `1015`); module paths a macro wrote are not references to the module (bevy's `resource` module: `1078` → `180`, rust-analyzer `183`); in a target root its own `#![cfg(…)]` switches off, paths that start at another crate resolve through the index (`tokio::test`: `881` → `979` of rust-analyzer's `988`). Suite against `0.3.1`: `definition` `1836` → `1838`, `documentHighlight` `1747` → `1755`, `references` `1593` → `1594`, no row lost; references p50 `0.66 ms` → `0.66 ms`, p95 `14.6 ms` → `15.2 ms` on the release builds (the commit message of `bcc1a901` gives `0.67` / `14.6 ms`, measured before its last change).

### Fixed
- **`textDocument/typeDefinition`**: a bare name the file declares as a type parameter (`impl<I, O> … for E<I, O>`) is no longer looked up by name across the workspace; parity `1223` → `1224` of `1308`, median latency `0.54 ms` → `0.55 ms`.

## [0.3.1] - 2026-11-11

Figures: the 38-method, 7-corpus suite, before (`0.3.0` with its driver) and after (the `0.3.1` release build), both indexed afresh with the same Mitts build. Indexes built by `0.3.0` should be rebuilt (`ridx index .`): the driver now records build scripts and trait functions named by paths.

### Changed
- **`textDocument/references`** on a trait item also lists its uses the compiler resolved to an implementation (`v.into_iter()` on a `Vec`, `x.clone()` on a type deriving `Clone`), as rust-analyzer does; the uses a macro generates (a derive's body) are left out. Associated types named through `Self` or a type parameter in signatures (`-> Result<V::Value, Self::Error>`) and in `Trait<Assoc = T>` bounds are references to the trait's associated type. Parity `80.26%` → `83.62%` (`1593/1905`); `documentHighlight` `92.20%` → `93.32%`, incoming calls `91.07%` → `91.75%`; median latency unchanged (`0.63 ms` → `0.66 ms`).
- **Indexing**: build scripts of the workspace's own packages are indexed; a trait function named by a path in a body (`From::from(x)`) refers to the implementation the arguments select.
- **`textDocument/semanticTokens`**: a target root its own `#![cfg(…)]` switches off, and an item an outer `#[cfg(…)]` leaves out of the build, get rust-analyzer's tokens (definitions typed; the names only that code defines or imports, method calls, fields and uses of locals `unresolvedReference`), and names inside macro arguments in doc examples are `generic`. Parity: full `91.90%` → `94.18%`, range `88.48%` → `90.67%`, delta `92.42%` → `94.66%`; median latency `0.45 ms` → `0.48 ms`, p95 `7.2 ms` → `8.3 ms`.
- **`workspace/symbol`** kinds follow rust-analyzer's symbol index (associated functions are functions, associated types type parameters, statics constants, crate roots packages): answers identical to rust-analyzer's `643` → `994` of `2100`.

### Fixed
- **Indexing with Mitts**: a target with `required-features` that the main check already built keeps its shard and sidecar, and the extra check's wrapper cache is scoped to the run, so a re-index no longer drops the targets' shards. On `clap` the references of `Command::long_about` found from a test file went from `15` to `42`, as from its definition.

## [0.3.0] - 2026-11-11

Live type inference behind the LSP: dependencies and the standard library come from the rustc-built index, while the workspace's own code is parsed, name-resolved and type-inferred by `ridx` itself, on saved and on edited buffers alike. Figures below are from the 38-method, 7-corpus content-parity suite against rust-analyzer (`serde`, `ripgrep`, `tokio`, `rust-analyzer`, `bevy`, `clap`, `regex`), before (the last state without live inference in the LSP) and after (`602f4a19`), on the same index.

### Added
- **`textDocument/typeDefinition` from the inferred type**: every type the inferred type names (`Vec<Foo>` leads to `Vec`, `Foo` and the allocator). Parity `89.22%` → `93.43%`.
- **`textDocument/completion` after `.` from the receiver's inferred type**: fields and methods of the full receiver type with autoderef, traits in scope, bounds of type parameters and of a trait's `Self`, `#[doc(hidden)]` items of other crates hidden, postfix snippets (`let`, `letm`, `for`, `match`, `ifl`, `lete`, `while`, `if`, `not`, `break`, `drop`) chosen by the receiver's type and the expression's position as rust-analyzer does, and `await` / `await.member` on futures. Parity `78.36%` → `95.52%` (`1920/2010`); exact label-set matches `292` → `1326`; median latency `2.90 ms` → `1.76 ms`.
- **`#[cfg(test)]` code answered from the unit that compiles it** (a library's test harness), for type queries and completion.

### Changed
- **`textDocument/references`**: `Self` inside an impl counts as a reference to its struct, enum or union. Parity `80.37%` → `82.20%`.
- **`textDocument/inlayHint`**: recorded types are merged from every unit compiling the file, so test-module bindings get hints. Parity `94.31%` → `95.08%`.
- **Trait solving**: bounds that land on a type parameter are checked against that parameter's declared bounds; compiler-implemented traits without an indexed definition (`Destruct`) no longer fail bound checks; type references no longer resolve to a same-path derive macro instead of the trait.

### Notes
- Refactoring assists stay compile-safe: `Inline into all callers` is not offered where the inlined body would reference private items from another module or crate, although rust-analyzer offers it there (`code_action` parity stays at `58%`).
- Latency: `typeDefinition` median `0.54 ms` (`0.16 ms` before; the inferred answer costs a parse and a function inference on the first query in a file). Peak server memory on `bevy` is unchanged at about 1 GB.

## [0.2.0] - 2026-10-30

First tagged release (`git tag` had no prior tags; entries below summarize changes relative to the initial `0.1.0` workspace baseline and the 5-corpus parity program `3fce845..d7fbd78`).

### Added
- **Drop-in `rust-analyzer` CLI & Editor Configs (`e9f468c`)**: Invoking `ridx` with no subcommand starts the LSP server on `stdio`. Added `rust-analyzer`-compatible CLI flags (`-v`/`--verbose`, `-q`/`--quiet`, `--log-file <path>`, `--print-config-schema`), configuration templates in [`docs/editors/vscode-settings.json`](docs/editors/vscode-settings.json), [`docs/editors/zed-settings.json`](docs/editors/zed-settings.json), and [`docs/editors/neovim-lspconfig.lua`](docs/editors/neovim-lspconfig.lua), and the headless drop-in session verifier ([`bench/dropin_editor_session.py`](bench/dropin_editor_session.py)).
- **Enriched `textDocument/hover` with Provenance & Blast Radius (`c2aaec7`)**: Hover responses now include full multi-line item declaration signatures (`where` clauses and bounds), source doc comments (`///`, `#[doc = "..."]`, `#[cfg_attr(..., doc = "...")]`), canonical crate/file **Provenance**, and precomputed $O(1)$ **Blast Radius** counters (`BlastRadiusIndex`) deduplicated by normalized workspace file path and Cargo package.
- **`.ridx` v3 Format (`DefEntry::owner_hash` & `DerefEntry`) (`d2cbf33`, `3af1467`)**: Extended binary index format (`.ridx` v3) with container `owner_hash` for associated items (`d2cbf33`) and `DerefEntry` receiver chains for `Deref` and supertrait method resolution (`3af1467`).
- **5-Corpus Content-Parity Sidecars (`7715510`, `462fe99`, `d7fbd78`)**: Published 5-corpus content-parity sidecars (`bench/published/position_ab_parity_*_vs_ra_ram9_*.differing.json`) across `serde`, `ripgrep`, `tokio`, `rust-analyzer`, and `bevy`.
- **Preparation for Sysroot Indexing**: Standard library path mapping (`std_path_candidates` mapping `std::` queries to canonical `core::` and `alloc::` paths in `find_definition_by_path` and `search_symbols`).
- **Rename Correctness Oracle**: Automated oracle verification against real-world crates (e.g. Tokio), validating that generated `WorkspaceEdit`s compile cleanly with zero errors (`bench/published/rename_oracle.json`).

### Changed
- **`textDocument/references` Precision & Container Scoping (`7748b2d`, `2a58115`)**: Filtered trait/impl sibling associated items by `DefEntry::owner_hash` and rejected multi-line and whole-item spans in cursor/reference resolution (`2a58115`), raising 5-corpus workspace line precision from `57.0%` (`030b224`) to `90.4%` (`152,976 / 169,270` lines, `83.5%` line recall, `968/1352 = 71.6%` full-set parity in `d7fbd78`).
- **`textDocument/definition` & `textDocument/hover` Parity (`75c8a64`, `7748b2d`, `c2aaec7`, `d7fbd78`)**: Reached `94.8%` (`1293/1364`) on `definition` and `96.1%` (`1332/1386`) on `hover` across the 5-corpus benchmark suite (`d7fbd78`), while avoiding redundant `type_at` scans when a resolved `DefEntry` already carries a signature (`7748b2d`).
- **`textDocument/completion` Type-Directed Receiver Lookup (`d2cbf33`, `3af1467`)**: Added owner-hash receiver member lookup (`d2cbf33`) and `Deref`/supertrait receiver chain traversal (`3af1467`), achieving `1108/1443 = 76.8%` target label recall (`878/1443 = 60.8%` in top-20, `37/1443 = 2.6%` exact full-set parity in `d7fbd78`).
- **selectionRange chain**: The innermost node is the token under the cursor (identifier, keyword, `r#raw`, lifetime, or the whole comment or string/char literal the cursor is inside); the `fn` body `{..}` and the `fn` item (including its outer attributes and doc comments, like `rust-analyzer`'s `FN` node) are separate nodes.
- **Canonical Path Extraction**: Wrapped reference and definition path generation in `ridx-driver` with `with_no_visible_paths!` and `with_no_trimmed_paths!`, eliminating re-export path divergences across multi-crate boundaries.
- **Re-export Alias Isolation**: Re-export alias names are strictly isolated from canonical references and used only for unmerged alias safety refusals during rename.
- **Notice on Reindexing**: Upgrades affecting driver path generation or `.ridx` v3 layout require reindexing workspaces (`ridx index .` or `ridx index . --all-features`) to replace cached CAS shards.

# Changelog

All notable changes to `rust-intelligence` (`ridx`) are documented in this file.

## [Unreleased]

### Fixed
- **An edit compiles the crates depending on it only when it changes what they see** (issue #7). Each crate others depend on has an interface: its tokens without comments (doc comments, which a macro or a derive reads, stay), without the bodies of ordinary functions and without positions. A body edit, a comment, or lines added above a definition leave it as it was: the dependents are not compiled, and their references to the crate's definitions are moved to where those now are, in their shards. The bodies a dependent sees into stay in the interface: a `const fn`, an `async fn`, a function returning `impl Trait`, a body holding an `impl` or calling a macro other than the standard library's, and every item under an attribute that may be a procedural macro; a macro call is kept whole, and an item under `#[cfg(test)]` is left out. A reference that cannot be placed compiles its crate as before. A dependent not compiled keeps metadata built against the crate's older build, which rustc refuses next to the newer: it is compiled before any crate compiled against both. The first edit after a Cargo index is compared too (the interfaces are recorded after it, about 0.2 s for ridx's 65 units). On ridx's own workspace (release build, Linux x86_64, the same `ridx-driver`, two runs per binary, `ridx index` times against 0.3.15): a comment appended to `ridx-format` 10.1–15.8 s to 2.0–2.2 s (10 units compiled to 2), a second one 7.5–8.8 s to 2.1–2.3 s, a line added inside a body of `ridx-format` (the definitions below it moving) 10.6–21.4 s to 2.2–2.3 s, an edit of `ridx` itself 9.1–17.7 s to 5.3–6.0 s. After those edits every workspace shard's references (1,339,676 in 65 shards) equal those of a Cargo index of the same text.

## [0.3.15] - 2026-11-26

Verified by the workspace test suite and by the e2e tests of each fix: completion of built-in types with and without an index, a workspace whose dependency fails to compile, and the first edit after a Cargo index. The index format is unchanged.

### Fixed
- **Built-in types are completed where a type is typed**: `bool`, `char`, `str`, the integer and the float types come first, ordered by width (`u8` before `u128`), after `let x: u`, a parameter's or a closure parameter's `:`, `->`, `as`, `const`/`static` items, `fn(u`, `impl Fn(u`, and inside a tuple or an array type; also in a generic argument (`Vec<u`, `HashMap<String, u`, on the next line too), a struct's field and a struct literal's field. They come with no index too, in a crate that did not build. Where only a type can follow, functions, macros, constants and locals are left out; where a value may follow as well (`Vec<u` may be a const argument, `S { a: u` a value), nothing is. Before, `let x: u` offered `unreachable!`, `unimplemented!` and the workspace's names, and no `u8`.
- **A dependency that does not compile no longer leaves its dependents unindexed**: when a registry or git dependency's library or procedural macro fails, `ridx-driver` compiles an empty crate of the same name in its place, so Cargo goes on to the crates using it; they are indexed, their uses of that crate unresolved. `ridx index` names the crates standing in. On serde 69072f24, whose `test_suite` dev-dependency `trybuild` fails after a fresh resolve, the 18 test targets are indexed and 34 positions with no answer before resolve as rust-analyzer's do.
- **The first edit after a Cargo index is compiled from Cargo's incremental cache**: the direct path used an empty cache of its own and `--emit=metadata` in place of Cargo's `--emit=dep-info,metadata`, which makes rustc ignore a cache. It now emits `dep-info` as Cargo's check does and uses the cache each unit was last compiled with, when that lies under the workspace's `target`. On ridx's own workspace (release build, Linux x86_64, two runs per binary), a comment appended to `ridx-format`, 34 units compiled: 20.0–20.1 s to 7.8–7.9 s (issue #7). The second cache under `target/ridx-direct/incremental` (about 600 MB there) is no longer filled.
- A unit compiled by the direct path records the direct search path once: each run added another `-L` to its recorded arguments.

## [0.3.14] - 2026-11-25

Verified on GitHub's runners for each released platform: the release build indexes a small crate, `ridx def` resolves a method call, and an LSP session (initialize, didOpen, definition, hover and references of a method call, shutdown, exit) answers with the file URIs each client writes; on Windows x86_64 and Intel macOS the ridx-format, ridx-syntax, ridx and ridx-driver tests pass. The index format is unchanged.

### Added
- **Windows x86_64 and Intel macOS**: release archives `ridx-x86_64-pc-windows-msvc.zip` and `ridx-x86_64-apple-darwin.tar.gz` (signed and notarized, as the Apple Silicon build), a PowerShell installer (`irm https://rust-intelligence.com/install.ps1 | iex`, into `%USERPROFILE%\.local\bin`, added to the user's PATH), and `install.sh` for Intel Macs. On Windows, ridx finds `ridx-driver.exe`, gives it the compiler's DLLs on `PATH`, finds the rustup toolchain under `%USERPROFILE%\.rustup`, replaces a shard while the server holds it mapped (opened with delete sharing), reads `file:///c%3A/...` and `file:///C:/...` URIs as `C:\...` paths and answers with `file:///C:/...`, recognises Cargo's registry by a path with either separator, and tells a live server's session from an ended one with `tasklist` (Git's `kill` there knows only MSYS processes and swept live sessions).
- **The Zed extension installs ridx itself** (0.2.0, id `rust-intelligence-language-server` as Zed's registry asks of an extension that only provides a language server): with no `lsp.rust-intelligence.binary.path` and no `ridx` on the PATH, it downloads the latest GitHub release for the platform into its own directory and runs it from there, removing older versions. Checked in Zed 1.30.2 on macOS arm64 with `ridx` taken off the PATH: release 0.3.13 downloaded, the crate indexed by the downloaded `ridx-driver`, hover answered.

### Fixed
- **An edit that breaks the build is reindexed without Cargo**: the crate the compiler stops on is indexed as it is, as Cargo's check indexes it, the crates depending on it keep their last index, and the edit fixing it is compiled the same way. Before, both edits fell back to a full `cargo check`. On ridx's own workspace (release build, Linux x86_64), a type error or a missing module file appended to `ridx-format`: 3.8–4.1 s to 1.7–1.8 s, and the edit reverting it 8.8–10.4 s to 6.2–7.8 s (issue #7). A unit is compared with the hashes of its own sources only: those it recorded for a crate it imports could hold an older text and hide a change.
- A release binary on a machine without the Rust toolchain `ridx-driver` runs inside says so, with the command installing it (`Rust 1.98.1 is not installed … rustup toolchain install 1.98.1 --profile minimal --component rust-src`), in `ridx index` and at the end of the editor's indexing progress. Before, the editor showed only `cargo check exited with status 101`.
- The SCIP loader of `ridx compare` reads every range encoding of the current SCIP schema: the typed `single_line_range` and `multi_line_range` (taking precedence when set), and the deprecated `range` of three or four elements. Before, an occurrence with only a typed range was skipped, and a four-element range was read with its end line as the end column. One small index per encoding is in `crates/ridx/tests/fixtures/scip/`.

## [0.3.13] - 2026-11-23

Verified by the workspace test suite and by an LSP session against the release build on a Linux node: a completion asked right after a fresh crate was opened (its build script sleeping 4 s) answered after its first index with `HashMap` and `use std::collections::HashMap;`. The index format is unchanged.

### Fixed
- **Requests sent before a fresh crate's first index**: a request at a position in a document whose root is being indexed for the first time, with no index holding it, waits for that index (up to 30 s) and is answered from it, as rust-analyzer answers once loaded. Before, completion of `HashMa` in a crate just opened answered `[]` (issue #8). Past the deadline, or when the first index fails, the request is answered as before, never with an error.
- A completion item that comes with its import also carries it as `data.imports: [{full_import_path, imported_name}]`, in rust-analyzer's shape.

## [0.3.12] - 2026-11-23

Verified by the workspace test suite (52 suites) and by LSP sessions against builds on Linux nodes: the issues prod-code filed from its live tests (#1 to #6) each with a regression test; `buildScripts: false` indexed a crate whose build script and derive leave marker files, with no marker written; a `minVersion` pull of `pub fn label() -> String { store::fresh() }` answers `mismatched types: expected `String`, found `u32``. The per-method agreement suite was not re-run for this release. The index format is unchanged.

### Added
- **`ridx/status` says when a change is not indexed yet**: `roots[].pendingChanges` is true from a save or a watched-file change until an index read after it is loaded. A change is told by the file's content, not its mtime: a file rewritten with its old mtime was not reindexed, and references answered old positions. Declared as `methods["ridx/status"]: 2`. Asked for by prod-code (issue #5).
- **Indexing with no build script and no procedural macro run**: `initializationOptions: {"buildScripts": false}` (declared as `capabilities.experimental.ridx.initializationOptions.buildScripts: 1`), `ridx index --no-build-scripts` or `RIDX_NO_BUILD_SCRIPTS=1`. Build scripts compile from `fn main() {}`, procedural macro crates from stubs declaring the same macros, in a target directory and an index variant of their own; the compiler's diagnostics are not given in this mode (`diagnosticsUnavailable`, `buildScriptsOff`). See docs/integration.md §7. Asked for by prod-code.
- **`workspace/symbol` over the libraries**: when nothing of the workspace matches, the dependencies and the standard library are searched, as rust-analyzer falls back to them: `Extend` lists `core::iter::Extend` (issue #6).
- **`minVersion` on `textDocument/diagnostic`** waits for the compiler's diagnostics of that version of the document, not for the index: a session compiles that very text (a buffer as saved too, for the request), and an empty answer then means clean. A version whose compile gave no diagnostics (stopped before checking the file, or a crate compiled with `--remap-path-prefix` or an `@file`) is answered with `-32803` `diagnosticsUnavailable` and the reason, never with an empty list. The compile is of the innermost workspace holding the file, and a pull held while a nested folder holding it is added is compiled there. `ridx/status` reports the same per document (`document.diagnostics`, and `document.building` of that workspace). Declared as `capabilities.experimental.ridx.methods["textDocument/diagnostic"]: 1`, which 0.3.10 and 0.3.11 lack. Asked for by prod-code, whose edit validation read the empty answer before the compile as clean.
- **New files in unsaved edits**: a module file that exists only as a buffer, included by another edited buffer (`mod moved;`, `#[path = "../tests/unit/moved.rs"]`), is compiled with it and reported; a new integration test (`tests/<name>.rs`, `tests/<name>/main.rs`) of a package that leaves its tests to Cargo's discovery is compiled as the target Cargo would build from it, from the arguments of a test harness of its package, with the package's edition; one of a package whose manifest configures its tests gets `diagnosticsUnavailable` with `targetUnresolved`. An edited buffer no crate includes gets `diagnosticsUnavailable` with `notInAnyCrate` at once; a file as saved that no crate includes gets the hint `unlinked-file` (severity 4) rust-analyzer gives, not an empty list; pushed diagnostics carry it too, for any buffer no crate includes.
- **A stable id on every code action** in `data.id`: rust-analyzer's assist id where the assist is the same (`add_explicit_type`, `extract_variable`, `generate_impl`, ...), with no index, for tools that ask for an assist by name (docs/integration.md §6). Asked for by prod-code.
- **`ridx/safeDelete {textDocument, position}`**: the edit deleting the item named there (with its doc comments and lines) when nothing in the workspace uses it, else `{refused: true, reason, remainingUses}`; a trait's or a trait impl's item, a `mod m;`, an entry point, an item the linker or the compiler uses, a dependency's source, an index of another text, an unsaved use elsewhere and an item the index does not know are refused with their reason; an inline module is checked with every declaration in it. It refuses with `staleIndex` while the index is behind the workspace's sources, told by their content: a file it read differs now, or a Rust file it did not read (added since, a `#[path]` module) names the item or calls a macro whose body does. Declared as `methods["ridx/safeDelete"]: 2`. Asked for by prod-code's safe delete and prune tools.
- **`ridx/expandMacro {textDocument, position}`**: `{name, expansion}` of a `macro_rules!` call expanded fully, or `null`. Declared as `methods["ridx/expandMacro"]: 1`; prod-code's convert-to-method reads it.
- **`ridx/syntaxTree {textDocument, range?}`**: the buffer's syntax tree in the JSON rust-analyzer's `viewSyntaxTree` gives, lossless, with rust-analyzer's kind names, for tools that read rust-analyzer's tree. Declared as `methods["ridx/syntaxTree"]: 1`. Asked for by prod-code's introduce-variable and convert-to-method tools.
- **New assists**: `Inline variable` (`inline_local_variable`), `Extract into function` (`extract_function`, for a whole expression), `Promote local to constant` (`promote_local_to_const`), `Wrap return type in Option` / `in Result` (`wrap_return_type_in_option`, `wrap_return_type_in_result`). Each is offered only where its result compiles and means what the code meant, narrower than rust-analyzer's: inline and promote take no operator (`let area = size * scale` is neither inlined nor promoted: moving it changes the order of evaluation or of an overflow, and a constant evaluates it at compile time), and inline reads only locals whose type is written; wrap takes no function with a macro, an attribute or the name `main`, in a package Cargo resolves to the 2018 edition or later. rust-analyzer's also inlines a value moved twice or into a loop, promotes an initializer that reads parameters, and wraps the return type of a function whose callers then break.

### Fixed
- A position request whose line is past the document's end fails with `-32602 InvalidParams` (`Invalid position L:C: the document has N lines`) instead of answering `null`; a column past a line's end is the line's end, as the protocol says (issue #2).
- Completion after `self.` offers the fields and the methods taking `self` that the document declares for the impl's type, also in a file no index holds; it answered nothing there (issue #3).
- A compiler diagnostic gives its primary label on the first line, as `cargo check --message-format=short` prints it: `mismatched types: expected `String`, found `u32`` (issue #4).
- `extract_function` is offered in a file with `mod tests { use super::*; }`: an import counts only in its own module, and a statement macro only in the block around the selection (issue #1).
- An edit is reindexed faster: the changed crate and its dependents compile side by side, and a second edit no longer falls back to a full `cargo check` (a comment edit in ridx's own workspace: 28 s to 18 s, later ones 6 to 8 s; issue #7, first part).
- `workspace/symbol` gives an item of a crate root its crate as `containerName` (`smoke` for `smoke::Config`), as rust-analyzer does; it was empty.
- `textDocument/implementation` places an impl at its self type (`Circle` of `impl Shape for Circle`), as rust-analyzer does, not at the `impl` keyword.
- A standard library file is named by its path in the toolchain's `rust-src` everywhere (call hierarchy, type hierarchy, symbols), not `file:///library/...`, which led nowhere.

## [0.3.11] - 2026-11-21

Verified by the workspace test suite and by LSP sessions against the release build on a Linux node: a wrong return type, an unresolved name and a wrong argument count typed and never saved are pushed and pulled as `rustc` diagnostics with the document's version, and cleared when the text is reverted or the buffer closed; hover on a parameter shows `on: bool` with `size = 1, align = 0x1, no Drop`. Released after ten review rounds of the session diagnostics. The per-method agreement suite was not re-run for this release. The index format is unchanged.

### Fixed
- **Errors in unsaved text**: what the compiler reports when a session index compiles the edited buffers (a wrong return type, an unresolved name, a wrong argument count, every error and lint) is published for those buffers, in the buffer's coordinates with the document's version, pushed and pulled, in place of the last check of the saved files (other files, and crates compiled with `--remap-path-prefix`, keep that check). 0.3.10 reported in an unsaved buffer only the type mismatches it found itself, and the compiler's errors after a save.
- **Hover on a binding** (a parameter, a `let`, a closure's parameter, a name in a pattern) shows `name: Type` with its layout (`size = 1, align = 0x1, no Drop`) and the name's range, as rust-analyzer does; a parameter showed `Type: bool`.
- **Diagnostics of a file whose name or URI needs escaping** (`a b.rs`, a file named `a%20b.rs`), or shown under a symbolic link: the check's result is looked up by the file's path and published under the URI the client knows it by, and cleared under every other URI it was shown under.

## [0.3.10] - 2026-11-20

Verified by the workspace test suite and by an LSP session against the release build on a Linux node with `ridx-driver` missing: the progress `end` names the failure, `references` is refused with `indexFailed`, `documentSymbol` is answered. The per-method agreement suite was not re-run for this release. The index format is unchanged.

### Fixed
- **A workspace whose index failed** no longer answers `[]`: with no index loaded and its last run failed, `definition`, `declaration`, `typeDefinition`, `implementation`, `references`, `rename`, `prepareRename`, `prepareCallHierarchy` and `prepareTypeHierarchy` are refused with `-32803` and `data: {reason: "indexFailed", root, error}`, also for a request already waiting for `minVersion`. Requests also answered from the syntax tree are answered; an index published since, or an older one loaded, answers.
- **Why an index failed**: `ridx/status` and the progress `end` give the last error line of the run's output (a missing `ridx-driver`, a manifest that does not parse) and where the rest is; the `end` names every root whose run failed under it, a short one included.
- **An index opened by where it is kept** (`--index-path` on `target/ridx`, a variant directory, an index kept outside the workspace) answers for the workspace it records, also next to an enclosing workspace, and is removed by the path it was opened by without touching other roots.
- **README**: the Known limitations, Why and How it works sections describe 0.3.9's indexing of crates with errors and of unsaved code (they still described 0.3.8), with the 0.3.9 citation, in all translations.

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

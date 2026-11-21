# Integrating with Rust Intelligence

This is the contract for tools that use Rust Intelligence as their Rust backend: editors, code-intelligence
services, agents. There is one public interface, the Language Server Protocol, extended by versioned
`ridx/*` methods documented here. No integration uses anything private, including the ones we build
ourselves.

Status in this document: **available** means implemented and covered by tests (**available since
<version>**: first in that release); **next release** means on `main` and not in a published release yet; **in progress** means being built now;
**planned** means agreed and not started. A method that is not marked available may change.

## 1. Interfaces

| Interface | Use it for | Status |
|---|---|---|
| `ridx lsp` (stdio) | Editors and interactive tools. Standard LSP plus the `ridx/*` methods below. | available |
| `ridx def / refs / type-at / hover / symbols / inspect --json` | Scripts and batch jobs. One query per process on a built index; each call loads the index. | available |
| `ridx index <dir>` | Building or refreshing the index of a Cargo workspace outside a server. | available |

`ridx lsp` accepts rust-analyzer's command-line flags (`--log-file`, `-v`, `--stdio`, ...), so a client
configured for rust-analyzer can start it unchanged.

## 2. Lifecycle and readiness

- On `initialize`, every workspace folder that holds a Cargo workspace without an index is queued, and
  indexing starts on `initialized`.
- While an index is built, the server reports work-done progress with a token starting `ridx-index-`
  (`begin`, `report` naming the roots, `end`). `end` means the **attempts ended**, not that a new index was
  loaded: a run that failed (an invalid `Cargo.toml`, a build script that fails) ends it too and leaves the
  index as it was. A successful run's index is normally loaded before its `end` is sent, but this is not
  guaranteed. There is one token for all folders being indexed, not one per folder. **available**
  To tell a loaded index from a failed run, per folder, ask `ridx/status` (§7) after `end`: each root's
  `generation` changes when a new index loads, and `error` says why its last run failed. **available since
  0.3.9** (there `error` names only the log; the last error line of the run's output in it: **available since 0.3.10**).
  `end` alone never means success: when a run under it failed, its `message` says so (`Indexing failed:
  <root>: <error>`, a run that started and failed between the server's looks included; **available since 0.3.10**), but
  only `ridx/status` tells per root which index is loaded.
- A root whose index failed and has none loaded (`indexed: false`, `error` set) refuses the requests the index
  alone answers (`definition`, `declaration`, `typeDefinition`, `implementation`, `references`, `rename`,
  `prepareRename`, `prepareCallHierarchy`, `prepareTypeHierarchy`) with `-32803` and
  `data: {reason: "indexFailed", root, error}`, so an empty list always means none. Requests also answered
  from the syntax tree (hover, completion, document symbols, formatting, ...) are answered. A root with an
  older index loaded answers from it, as does one whose index was published since by anyone; one building its
  first index answers as before (use `minVersion` to wait for it). A request already waiting for `minVersion`
  when the run fails gets the same error then, not a timeout. The root is the innermost workspace holding the
  file, so a nested workspace whose index failed is refused even inside a loaded outer one. **available since 0.3.10**
- A save, or a watched-file change of a source, starts a background reindex of the changed crates; the
  same progress token reports it. **available**
- `$/cancelRequest` is honoured for a request read and not answered yet (queued, or waiting for `minVersion`); one for a request answered already, or never sent, is ignored, so an id can be reused. **available** For long-running requests: **planned**.

## 3. Code that does not compile

A crate of the workspace with compile errors is indexed from everything that resolved: definitions,
references, types, implementations and calls in it are answered, including inside the bodies with errors;
an expression or binding whose type failed is left out. A registry or git dependency that fails to compile
gets no index. **available since 0.3.9**; 0.3.8 indexed only crates that compiled.

## 4. Unsaved text

The index is built by the compiler from the files on disk. For text that is open and edited but not saved:

| What | Status |
|---|---|
| Type diagnostics of the buffer (pushed and pulled), hover, completion and inlay hints on typed code | available |
| Navigation, references and rename on positions that exist in the saved text (mapped through the edit) | available |
| References, rename and navigation that include code typed since the last save | available since 0.3.9 |
| The compiler's diagnostics of the unsaved text: every error and lint rustc reports on it, in the buffer's coordinates | available since 0.3.11 |
| Files not on disk yet: a module an edited buffer includes, a new integration test (`tests/<name>.rs`) compiled as its target | next release |

How: once the edited buffers of a workspace have been unchanged for 250 ms, the crates whose own sources
include them (the driver records each crate's sources) are compiled again by `ridx-driver` from the buffers'
text (nothing is written to the files) into an index of this session under `target/ridx/sessions/`. Edited
crates are compiled in dependency order, and one reads the session's metadata of another edited crate it
uses, when that one compiled. The session index answers in place of the published one until a newer published
index loads (a save, or `ridx index` run by anyone), or until no buffer is edited any more (all reverted or
closed); then the published index answers again. Between an edit and that index, answers come from the
previous one, mapped through the edit; a tool that must not get such an answer asks with `minVersion` (§7).
Crates that depend on an edited one without being edited keep their saved-text index until the save.
A buffer whose crate the compiler cannot parse keeps the previous index of that crate and stays uncovered
(`coveredVersion` does not move, `minVersion` times out) until a text that parses is compiled. While a run
of `ridx index` holds the published index, no session index is built; the next one starts when it ends.
The compiler's messages of these compiles are the diagnostics of the session (§5).
`RIDX_SESSION_INDEX=0` turns it off.

Measured from `didChange` to `references` that include the edit (Linux x86_64 build node, release build,
10 edits each): a one-file crate p50 0.36 s, p95 0.36 s; a file of `ridx-format` in this repository's
workspace (5 packages, 65 indexed crates, its library and unit tests recompiled side by side) p50 3.31 s,
p95 3.55 s. The time is the compile of the crates owning the buffer: it grows with them, not with the
workspace.

## 5. Diagnostics

| `source` | What | When it is published |
|---|---|---|
| `rustc` | The compiler's own errors and lints (`code` = the rustc code, e.g. `E0433`, `unused_imports`, `dead_code`) | After a check of the workspace: on `initialized` (every workspace of the session, one after the other; since 0.3.9) and after every save. Ranges are in the coordinates of the file as saved; editors move them through unsaved edits. |
| `rustc` | The same, on the text of a session index (§4): every error and lint the compiler reports on the edited buffers it compiled | When a session index loads, for each buffer it compiled, in the buffer's coordinates with the document's version, in place of the check's for that file while a session has it (saved since or not). Other files, and the files of a crate compiled with `--remap-path-prefix`, keep the check of the saved files. Zed, which moves `rustc` diagnostics through unsaved edits itself, gets them as `ridx` (since 0.3.11). |
| `ridx` | Errors found in the unsaved text (type mismatches and similar) before the compiler sees it | On `didChange`, for the open buffer. |

Both are available. The values of `source` and the rustc `code` values are stable. The server pushes
diagnostics unless the client declares pull diagnostics (`textDocument/diagnostic`); then it answers pulls
only.

## 6. Code actions

The aim is that every code action produces code that compiles: the refactoring assists are written and tested
to that end, and an assist rust-analyzer offers whose result would not build is not offered. This is not a
check of each edit: ridx does not compile the result before offering it, so a caller that needs the
guarantee validates the result (as `ridx/diagnosticsFor` will allow). A quickfix that broke code is a bug
to report.

| What | Status |
|---|---|
| `textDocument/codeAction` with `kind` and title, `codeAction/resolve` | available |
| A stable id on every action in `data.id`, using rust-analyzer's assist id where the assist is the same (`extract_function`, `inline_local_variable`, ...) | **planned** (next after unsaved text) |
| `ridx/assists` / `ridx/applyAssist` by id | **planned**, same release as the ids |

The first ids, in this order: `extract_function`, `extract_variable`, `inline_local_variable`,
`inline_call`, `wrap_return_type_in_option`, `wrap_return_type_in_result`, `remove_unused_imports`,
`move_module_to_file`, `extract_module`, `auto_import`, `qualify_path`. An id, once published, keeps its
meaning in every later version.

The list of assists with their ids will be kept here when the ids land. An assist a tool needs and ridx
lacks is reported on the [issue tracker](https://github.com/alex09x/rust-intelligence/issues) and
implemented in order of need.

## 7. `ridx/*` extension methods

All take the session's unsaved text into account. Version: the server lists the methods it supports, with
their versions, in `initialize` → `capabilities.experimental.ridx`: `{methods: {"ridx/status": 1}, requestOptions: {minVersion: 1}}` (**available since 0.3.9**). A method or option missing there is not supported by that server; a version changes only with an incompatible change of its contract.

| Method | Params → result | Status |
|---|---|---|
| `ridx/status` | `{textDocument?}` → `{roots: [{root, indexed, generation, building, error, sessionIndex: {answers, building}}], document?: {version, coveredVersion, building}}`; a root still building its first index, or whose first index failed, is listed with `indexed: false`, `generation: null`; `coveredVersion` is the buffer version whose text the index answering for the file recorded, `null` while edits since are not compiled or when no index answers for it; the published index is reloaded first if it changed | available since 0.3.9 |
| `minVersion` on any request with `textDocument` | `params.minVersion: N` (the buffer's LSP version) → the request waits until the index answering covers version `N` of that document, or fails with `-32803` and `data: {reason: "timeout", coveredVersion}` once the timeout passed (even if the index arrives later); `$/cancelRequest` ends it with `-32800`, `shutdown` with `-32600`. Requests only: on a notification the field is ignored | available since 0.3.9 |
| `minVersion` on `textDocument/diagnostic` | The pull waits until the compiler's diagnostics of version `N` (or later) of the document are known, not the index: for an edited buffer, once a session index compiled that very text; for a buffer as saved, once the check of its workspace's saved files ran and none is queued. Then an empty `items` means clean. When the compile of that version gave no diagnostics, the pull fails at once with `-32803` and `data: {reason: "diagnosticsUnavailable", detail, version}`: `detail` is `notChecked` (no unit owning the file checked it to the end, e.g. a proc macro panicked; a parse error is reported as diagnostics) , `pathsUntold` (a unit owning it uses `--remap-path-prefix` or an `@file`) or `notInAnyCrate` (a file no crate includes, counting the edited buffers: a new file nothing declares). After the timeout: `-32803` with `reason: "timeout"` and `coveredVersion`, the version whose diagnostics are known. `ridx/status` gives the same as `document.diagnostics: {state: ready \| unavailable \| pending, version, reason?}` | next release |
| `ridx/diagnosticsFor` | `{texts: {uri: text}}` → `{uri: Diagnostic[]}`: what the compiler would report if these files had this text, without changing the session's buffers | planned (after `minVersion`) |
| `ridx/resolveSymbol` | `{path: "crate::module::Type::method", kind?}` → `Location[]` | planned |
| `ridx/typeAt` | `{textDocument, range}` → `{type, ...}` for an arbitrary expression range | planned |
| `ridx/symbols` | `{query, scope: "workspace" \| "dependencies" \| "all", kinds?, limit}` → `SymbolInformation[]` | planned |
| `ridx/safeDelete` | `{textDocument, position}` → `{edit}` or `{refused, remainingUses: Location[]}` | planned |
| `ridx/expandMacro` | `{textDocument, position}` → `{name, expansion}` | planned |
| `ridx/syntaxTree` | `{textDocument, range?}` → syntax tree | planned |
| `ridx/structuralReplace` | `{pattern, replacement?, scope}` → matches or `WorkspaceEdit` | planned |
| `ridx/batch` | `{requests: [{method, params}]}` → results in order | planned, optional |

What these methods promise, beyond the signatures:

- **Freshness.** Without `minVersion` a request is answered at once, from the newest index there is, mapped
  through the edits since (right for an editor). With it, the answer comes from an index that compiled at
  least that version of the document, so a tool that edits and then asks never gets a stale answer and
  never polls. A document whose text equals the saved file is covered by the published index once it
  has been indexed. The timeout is 30 s unless `params.minVersionTimeoutMs` says otherwise. A document that
  is not open has no version to wait for and is answered at once; one no indexed crate holds (a new file
  not yet in any `mod`) is never covered and times out. Measured on a one-file crate: `references` sent
  right after `didChange` with `minVersion` answered from the new text in 0.33 s.
- **`ridx/diagnosticsFor`** takes `{texts: {uri: text}, report?: uri[] | "affected"}`. Without `report`
  it reports the files passed in; `"affected"` adds every file of every crate it recompiled. With
  `"affected"` it recompiles the crates that depend on the edited ones too, so a changed declaration
  reports the errors at its uses in other crates (the type-migration case). A dependent of a crate that
  stopped with errors cannot be checked against the new text (rustc writes no metadata for it): it is
  reported `complete: false`, `reason: "dependency has errors"`. Ranges in the files passed in are in the coordinates of the text passed in, the
  exception to §5; other files are in their saved coordinates. The result carries, per crate,
  `{complete: bool, reason?}`: a crate whose build stopped before type checking, or whose proc macro failed,
  is `complete: false`, and its empty list must not be read as clean. Calls are isolated: concurrent calls
  from one session never see each other's texts, nor change the session's buffers. Its latency (p50 and
  p95, one edited file, on a mid-size workspace) is published here with the release that ships it.
- **`ridx/resolveSymbol`** returns every match: for `Type::method` each inherent and trait method, with its
  `containerName` and trait. It never picks one silently. Generic arguments in the path (`Wrapper<T>::get`)
  are ignored when matching.
- **`ridx/symbols`** gives each result its full module path as well as its location, and the result carries
  `incomplete: true` when `limit` cut the list.
- **`ridx/typeAt`** returns `{type, expected?}`; `expected` is the type the context requires at that range
  when it differs (a coercion site).
- **`ridx/syntaxTree`** returns rust-analyzer's JSON shape, `{kind, start: [offset, line, col], end: [...],
  children}`, lossless, the root `SOURCE_FILE` covering the whole text, with rust-analyzer's `SyntaxKind`
  names (`FN`, `BLOCK_EXPR`, `STMT_LIST`, `LET_STMT`, `NAME_REF`, `MACRO_CALL`, `TOKEN_TREE`, ...), so tools
  written against rust-analyzer's tree read it unchanged.

Standard methods still to complete: `workspace/willRenameFiles` with edits to `mod` declarations and `use`
paths (today it returns `{}`), and `workspace/symbol` over dependencies.

## 8. Where the index lives and who builds it

- The index of a workspace is `target/ridx/` under the workspace root; the check it runs uses
  `target/ridx_cargo/`. Shards of registry dependencies and the standard library are shared by every
  workspace through the cache directory (`$RIDX_CACHE_DIR`, default the platform cache directory).
  **available**
- One build at a time per index: a run takes the exclusive lock `target/ridx/.index.lock` and another run
  waits for it. Several `ridx lsp` processes may read the same index at once; each keeps only its own
  session's buffers. **available**
- Shards of workspace crates addressed by content (the crate's sources, cfg and features), so a new git
  worktree of a repository reuses the shards of every unchanged crate from a sibling worktree or the cache
  instead of building its index from scratch. **planned**
- Choosing the index and target directories from outside (`--index-dir` / `--target-dir` and an
  `initializationOptions` equivalent), so build output stays out of the source tree. **planned**

## 9. Running untrusted code: build scripts and proc macros

Indexing runs `cargo check` with `ridx-driver` as the compiler, so the workspace's build scripts and proc
macros run as in any build. They get the environment `ridx` was started with, plus what Cargo sets, with
these set or replaced by ridx:

- the index build (`cargo check`): `RIDX_OUT_DIR`, `RIDX_CACHE_DIR`, `RIDX_WORKSPACE_ROOT`,
  `RIDX_TARGET_KINDS_FILE`, `RIDX_CARGO_LOCK_HASH` (when there is a `Cargo.lock`), `RUSTC_BOOTSTRAP=1`,
  `CARGO_TARGET_DIR` (`target/ridx_cargo/<id>`, replacing the caller's), `RUSTC_WRAPPER` = `ridx-driver` and
  `RUSTC_WORKSPACE_WRAPPER` empty (the other way round when `RIDX_NO_DEPS` is set: only workspace crates go
  through the driver), `LD_LIBRARY_PATH` and `DYLD_FALLBACK_LIBRARY_PATH` (replaced by the toolchain's library
  directory, when it is found), and `RUSTUP_TOOLCHAIN` = the driver's toolchain unless the caller set one;
- a crate compiled directly after a save (no Cargo): the variables Cargo set for it in the last build, plus
  `RIDX_OUT_DIR`, `RIDX_CACHE_DIR`, `RIDX_WORKSPACE_ROOT`, `RIDX_TARGET_KINDS_FILE` (when the variant has
  one), `RIDX_CARGO_LOCK_HASH`, `RUSTC_BOOTSTRAP=1` and the library path variables above;
- a crate compiled for a session index (unsaved buffers, §4): the variables Cargo set for it in the last build
  without any `RIDX_*` one, then `RIDX_OUT_DIR`, `RIDX_CACHE_DIR`, `RIDX_WORKSPACE_ROOT`, `RIDX_BUFFERS`,
  `RIDX_TARGET_KINDS_FILE` (removed when the variant has none), `RUSTC_BOOTSTRAP=1` and the library path
  variables above;
- the diagnostics check: `CARGO_TARGET_DIR` (`target/ridx-flycheck`, replacing the caller's); with
  `RIDX_FLYCHECK_DRIVER=1` also `RUSTC_WRAPPER` = `ridx-driver`, `RUSTC_BOOTSTRAP=1`, the library path
  variables and `RUSTUP_TOOLCHAIN` as for the index build.

A caller that wants a sandbox starts `ridx` in it with a scrubbed environment.

| What | Status |
|---|---|
| The environment rule above | available |
| A budget for index builds shared by every workspace on a machine: concurrent jobs, then a memory cap | planned (before the wrapper) |
| A wrapper command for `ridx-driver` and its rustc children (as `RUSTC_WRAPPER`), for a sandbox launcher | planned |

## 10. Requirements

`ridx` and `ridx-driver` are built with Rust 1.98.1 and run the index build on that toolchain, which must be
installed with `rustc-dev` and `rust-src`. See the [README](../README.md#install).

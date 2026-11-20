# Integrating with Rust Intelligence

This is the contract for tools that use Rust Intelligence as their Rust backend: editors, code-intelligence
services, agents. There is one public interface, the Language Server Protocol, extended by versioned
`ridx/*` methods documented here. No integration uses anything private, including the ones we build
ourselves.

Status in this document: **available** means released and covered by tests; **in progress** means being
built now; **planned** means agreed and not started. A method that is not marked available may change.

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
  (`begin`, `report` naming the roots, `end`). **`end` is the readiness signal**: it is sent within 20 ms
  after the new index has been loaded, so requests after it are answered from that index. **available**
- A save, or a watched-file change of a source, starts a background reindex of the changed crates; the
  same progress token reports it. **available**
- `$/cancelRequest` is honoured for queued requests. **available** For long-running requests: **planned**.

## 3. Code that does not compile

A crate with compile errors is indexed from everything that resolved: definitions, references, types,
implementations and calls in it are answered, including inside the bodies with errors; an expression whose
type failed is left out. **available** (next release; 0.3.8 indexed only crates that compiled)

## 4. Unsaved text

The index is built by the compiler from the files on disk. For text that is open and edited but not saved:

| What | Status |
|---|---|
| Type diagnostics of the buffer (pushed and pulled), hover, completion and inlay hints on typed code | available |
| Navigation, references and rename on positions that exist in the saved text (mapped through the edit) | available |
| References, rename and navigation that include code typed since the last save | **in progress**: the changed crates are compiled from the buffers' text, without writing them, into an index of the session |

Until the last row is available, references on a dirty buffer leave out code typed since the save, and
`rename` there can refuse (`-32803`) rather than produce edits from the saved text.

## 5. Diagnostics

| `source` | What | When it is published |
|---|---|---|
| `rustc` | The compiler's own errors and lints (`code` = the rustc code, e.g. `E0433`, `unused_imports`, `dead_code`) | After a check of the workspace: on `initialized` and after every save. Ranges are in the coordinates of the file as saved; editors move them through unsaved edits. |
| `ridx` | Errors found in the unsaved text (type mismatches and similar) before the compiler sees it | On `didChange`, for the open buffer. |

Both are available. The values of `source` and the rustc `code` values are stable. The server pushes
diagnostics unless the client declares pull diagnostics (`textDocument/diagnostic`); then it answers pulls
only.

## 6. Code actions

Every code action ridx offers produces code that compiles: an assist rust-analyzer offers but whose result
would not build is not offered.

| What | Status |
|---|---|
| `textDocument/codeAction` with `kind` and title, `codeAction/resolve` | available |
| A stable id on every action in `data.id`, using rust-analyzer's assist id where the assist is the same (`extract_function`, `inline_local_variable`, ...) | **planned** (next after unsaved text) |
| `ridx/assists` / `ridx/applyAssist` by id | **planned**, same release as the ids |

The list of assists with their ids will be kept here when the ids land. An assist a tool needs and ridx
lacks is reported on the [issue tracker](https://github.com/alex09x/rust-intelligence/issues) and
implemented in order of need.

## 7. `ridx/*` extension methods

All take the session's unsaved text into account. Version: the server lists the methods it supports, with
their versions, in `initialize` → `capabilities.experimental.ridx` (**planned**, with the first method).

| Method | Params → result | Status |
|---|---|---|
| `ridx/diagnosticsFor` | `{texts: {uri: text}}` → `{uri: Diagnostic[]}`: what the compiler would report if these files had this text, without changing the session's buffers | planned (first) |
| `ridx/resolveSymbol` | `{path: "crate::module::Type::method", kind?}` → `Location[]` | planned |
| `ridx/typeAt` | `{textDocument, range}` → `{type, ...}` for an arbitrary expression range | planned |
| `ridx/symbols` | `{query, scope: "workspace" \| "dependencies" \| "all", kinds?, limit}` → `SymbolInformation[]` | planned |
| `ridx/safeDelete` | `{textDocument, position}` → `{edit}` or `{refused, remainingUses: Location[]}` | planned |
| `ridx/expandMacro` | `{textDocument, position}` → `{name, expansion}` | planned |
| `ridx/syntaxTree` | `{textDocument, range?}` → syntax tree | planned |
| `ridx/structuralReplace` | `{pattern, replacement?, scope}` → matches or `WorkspaceEdit` | planned |
| `ridx/batch` | `{requests: [{method, params}]}` → results in order | planned, optional |

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
- Choosing the index and target directories from outside (`--index-dir` / `--target-dir` and an
  `initializationOptions` equivalent), so build output stays out of the source tree. **planned**

## 9. Running untrusted code: build scripts and proc macros

Indexing runs `cargo check` with `ridx-driver` as the compiler, so the workspace's build scripts and proc
macros run as in any build. ridx passes them only the environment it was started with, plus the variables
Cargo sets and these of its own: `RIDX_OUT_DIR`, `RIDX_CACHE_DIR`, `RIDX_WORKSPACE_ROOT`,
`RIDX_TARGET_KINDS_FILE`, `RIDX_CARGO_LOCK_HASH`, `RUSTC_BOOTSTRAP=1`. A caller that wants a sandbox starts
`ridx` in it with a scrubbed environment.

| What | Status |
|---|---|
| The environment rule above | available |
| A wrapper command for `ridx-driver` and its rustc children (as `RUSTC_WRAPPER`), for a sandbox launcher | planned |
| A budget for index builds: jobs, and a memory cap | planned |

## 10. Requirements

`ridx` and `ridx-driver` are built with Rust 1.98.1 and run the index build on that toolchain, which must be
installed with `rustc-dev` and `rust-src`. See the [README](../README.md#install).

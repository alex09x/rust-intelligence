# Architecture

`ridx` is a Rust language server that does not analyze Rust itself for code that has been built. The
compiler already resolves every name, expands every macro and infers every type on each `cargo check`;
`ridx` keeps those answers. A rustc driver writes them to disk as compact binary shards, and the language
server answers editor requests by looking them up in memory-mapped files. Only the code typed since the
last build is analyzed by `ridx` itself, by a small compiler-free front end over the same index.

```
 build time                                   edit time

 ridx index                                   editor
   │                                            │ JSON-RPC (stdio)
   ▼                                            ▼
 cargo check                                  ridx lsp
   │ RUSTC_WRAPPER                              ├─ overlay: open buffers, positions
   ▼                                            ├─ live: names and types of unsaved code
 ridx-driver (rustc + after_analysis)           ├─ WorkspaceIndex ◄── mmap ──┐
   │ HIR + TypeckResults                        └─ flycheck, reindex         │
   ▼                                                                         │
 .ridx shards ───────────────────────────────────────────────────────────────┘
   ~/.cache/ridx/v5/objects/     dependencies and std, shared by every project
   target/ridx/variants/<id>/    the workspace's own crates + manifest
```

## Crates

| Crate | Role |
|---|---|
| [`ridx-driver`](../crates/ridx-driver) | A rustc driver (`rustc_driver::run_compiler` with an `after_analysis` callback). Visits the HIR and the type-check results of the crate being compiled and writes its shard. Links to the compiler's private libraries, hence the pinned toolchain. |
| [`ridx-format`](../crates/ridx-format) | The `.ridx` format: writer, zero-copy reader, and `WorkspaceIndex`, which joins the shards of a workspace, its dependencies and the standard library into one queryable view. |
| [`ridx-syntax`](../crates/ridx-syntax) | A zero-dependency lossless lexer and error-tolerant parser, local name resolution and type inference. Used only for code the index has not seen yet. |
| [`ridx`](../crates/ridx) | The binary: the `index` orchestration around Cargo, the LSP server, the live layer that connects `ridx-syntax` to the index, and the shell commands (`def`, `refs`, `hover`, `compare`, …). |

## 1. Building the index

`ridx index .` runs `cargo check` with `ridx-driver` as `RUSTC_WRAPPER`. Every crate Cargo compiles goes
through the real compiler; after analysis the driver gets the `TyCtxt` and extracts:

- **definitions** with their kind, span, name span, owner (the enclosing item) and canonical path;
- **references**: every resolved path, method call, field access, macro call and attribute, with the
  `DefId` it resolves to. Method calls and fields come from rustc's `TypeckResults`, so `x.len()` points at
  the impl that rustc selected, through auto-deref and trait resolution, not at a guess by name;
- **types** of definitions, fields, generic parameters, `where` predicates, impl headers and associated
  type bindings, lowered from `rustc_middle::ty::Ty` into a structural, hash-consed type arena;
- **macros** (`macro_rules!` bodies), `use` items, active `cfg`s, lang items, and short **notes** — texts
  the compiler knows about a span and hover shows as they are: the generic arguments of a call
  (`T` = `String`) and the memory layout of a type, field or binding (`size = 8, align = 0x8, no Drop`).

The driver then lets compilation continue normally, so the `cargo check` result is unchanged.

### Dependencies are indexed once per machine

A dependency is immutable once Cargo has downloaded it. Its shard goes to a content-addressed store,
`~/.cache/ridx/v5/objects/`, under a key built from the crate's name and version, its manifest
directory, the target triple, its `--cfg` flags, the **content digests** of the `--extern` artifacts it was
compiled against, the format and extractor versions and the driver's own hash. If a valid shard with that
key exists, the driver skips extraction. Two projects that use `serde 1.0.228` with the same features
share one shard; a second worktree of the same project indexes only its own crates.

The standard library (`core`, `alloc`, `std`) is indexed the same way from its metadata, once per
toolchain, so navigation and hover reach into `std` without the sources being analyzed by `ridx`.

### Workspace crates and variants

The workspace's own crates are written under `target/ridx/variants/<id>/` with a manifest that lists every
shard the workspace needs, its own and its dependencies'. The variant id is a hash of what changes the
compiled code: profile, target, features (`--features`, `--all-features`, `--no-default-features`), the
`Cargo.toml` files, `Cargo.lock`, the rustc version and `RUSTFLAGS`. Switching features or targets
produces another variant next to the first instead of overwriting it.

### A failed build keeps the last good index

When `cargo check` fails, the crates it did rebuild replace their shards and every crate it could not
build keeps its previous one. A syntax error in one file never empties the index of a project.

### Reindexing after a save

The server starts `ridx index` in the background after each save and after a source file changes on disk
(a checkout, a pull), one run per workspace at a time; changes made during a run are folded into one more
run. When nothing but workspace sources changed, a fast path calls the driver directly on the changed
crates and their dependents in topological order, with the arguments Cargo used last time, without
starting Cargo. It falls back to Cargo when the build configuration changed or a new `.rs` file
appeared. The server watches the manifest and swaps in the new index when it is published.

## 2. The `.ridx` shard

One shard per compiled crate. Format version 5, little-endian, every record fixed-size and 8-byte aligned,
so a table is an array that can be read in place:

| Section | Record | Used for |
|---|---|---|
| header | — | version, crate, unit key, target, features, section offsets |
| string pool | — | every name, path and doc text, referenced by offset |
| files | 16 B | source files with content hashes (an open buffer is matched to the text that was compiled) |
| definitions | 40 B | sorted, with an FNV-1a hash index by path and a position index by span |
| owners, deref | — | members of a type, `Deref` targets: completion after `.` and receiver chains |
| references | 48 B | sorted by span (what is under the cursor) with a hash index by target (find references) |
| type arena | — | structural types of definitions, fields and expressions |
| impls, assoc types | — | trait and inherent impls, `type Item = …` bindings: implementations, method lookup |
| fields, generics, where | — | struct and variant fields, generic parameters, bounds and supertraits |
| uses, macros, cfgs, lang items, notes | — | auto-import, macro expansion for live code, hover texts |

The reader validates the header and maps the file. A lookup is a hash probe or a binary search over mapped
pages; nothing is decoded into heap structures, and the file descriptor is closed once the file is mapped.
Pages that are not touched are never read from disk, and the kernel can drop the ones that were under
memory pressure and read them again later. This is why a `ridx` process on Bevy stays under 1 GB while
the index for the workspace and all its dependencies is on disk.

`WorkspaceIndex` opens the manifest, maps the shards lazily per crate, and adds the indices that span
crates: which unit a file belongs to, which crates can see which (for references across crates and
re-exports), the standard-library prelude, and the mapping of `std::` paths to their canonical `core::`
and `alloc::` definitions.

## 3. Answering a request

For most requests the server finds what is under the cursor in the reference table of the file's shard
and follows its target:

| Request | Answer from |
|---|---|
| definition, declaration | the reference's target definition; across crates through the target's canonical path |
| references, highlight, rename | the target hash index of every shard that can see the defining crate |
| implementation, type hierarchy | the impl table and supertraits |
| hover, type definition, inlay hints | the type arena, docs from the string pool, notes |
| call hierarchy | references inside function bodies, grouped by the enclosing definition |
| completion, signature help | owners and deref tables for `.` and `::`, the uses table for auto-import; live inference for unsaved code |
| diagnostics | `cargo check` on save (flycheck), messages mapped to the saved text |
| code actions | assists that are checked to compile before they are offered |
| formatting | `rustfmt` |

The driver's answers are rustc's own, so on code that has been built a definition or a type is what the
compiler decided, including through macros, `impl Trait`, trait objects and auto-deref.

## 4. Code typed since the last build

The index describes the text that was compiled. Between a keystroke and the next successful build the
server keeps two things for each open buffer:

- **The overlay** (`crates/ridx/src/overlay`) maps positions in the edited buffer onto the compiled text
  line by line, using the file hash recorded in the shard. Everything on an unchanged line is still
  answered by the index.
- **The live layer** (`crates/ridx/src/live` on top of `crates/ridx-syntax`) analyzes the edited code
  itself: a lossless lexer and an error-tolerant parser, name resolution over the crate's module tree
  (`use`, globs, re-exports, `cfg`, `macro_rules!`), and local type inference with unification, coercion,
  auto-deref, method resolution and a trait solver. Every type and item outside the edited code — a
  dependency, `std`, another file of the crate — is read from the index through an adapter
  (`RidxV5ItemDb`), so the live layer never analyzes a whole crate.

When the next index arrives, each buffer is matched to it through the recorded hash, and the overlay
starts over from the new compiled text.

## 5. Trade-offs

- **The index is built by `cargo check`.** The first build of a large workspace takes minutes; after that
  only changed crates are rebuilt, and dependencies already in the store are skipped.
- **One configuration at a time.** Like `cargo check`, the index sees one target and one feature set.
- **The driver is tied to one toolchain** because it links to rustc's private crates. `ridx` runs the index
  build with that toolchain whatever toolchain the project uses; the installer adds it.
- **Live analysis is narrower than the compiler.** It covers what is typed between builds; once the code
  builds, the compiler's answer replaces it.

## Where to read the code

| Topic | Path |
|---|---|
| Driver entry and HIR visitor | `crates/ridx-driver/src/main.rs`, `hir_visitor.rs`, `build_index/` |
| Dependency store key | `crates/ridx-driver/src/unit_key.rs` |
| Type lowering | `crates/ridx-driver/src/ty_lower/` |
| Standard library shards | `crates/ridx-driver/src/sysroot/` |
| Shard layout | `crates/ridx-format/src/format.rs`, `format/` |
| Reader and workspace view | `crates/ridx-format/src/reader.rs`, `workspace/` |
| Index orchestration, variants, fast path | `crates/ridx/src/indexer/` (`variant.rs`, `last_good.rs`, `direct/`) |
| LSP server | `crates/ridx/src/lsp/` |
| Overlay and live analysis | `crates/ridx/src/overlay/`, `crates/ridx/src/live/`, `crates/ridx-syntax/` |

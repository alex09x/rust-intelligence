# Architecture

`ridx` is a Rust language server that does not analyze Rust itself for code that has been built. The
compiler already resolves every name, expands every macro and infers every type on each `cargo check`;
`ridx` keeps those answers. A rustc driver writes them to disk as compact binary shards, and the language
server answers editor requests by looking them up in memory-mapped files. Only the code typed since the
last build is analyzed by `ridx` itself, by a small compiler-free front end over the same index.

![How ridx works](../site/assets/architecture.svg)

## Crates

| Crate | Role |
|---|---|
| [`ridx-driver`](../crates/ridx-driver) | A rustc driver (`rustc_driver::run_compiler` with an `after_analysis` callback). Visits the HIR and the type-check results of the crate being compiled and writes its shard. Links to the compiler's private libraries, hence the pinned toolchain. |
| [`ridx-format`](../crates/ridx-format) | The `.ridx` format: writer, zero-copy reader, and `WorkspaceIndex`, which joins the shards of a workspace, its dependencies and the standard library into one queryable view. |
| [`ridx-syntax`](../crates/ridx-syntax) | A zero-dependency lossless lexer and error-tolerant parser, local name resolution and type inference. Used for code the index has not seen yet and for what the index does not record. |
| [`ridx`](../crates/ridx) | The binary: the `index` orchestration around Cargo, the LSP server, the live layer that connects `ridx-syntax` to the index, and the shell commands (`def`, `refs`, `hover`, `compare`, …). |

## 1. Building the index

`ridx index .` runs `cargo check` with `ridx-driver` as `RUSTC_WRAPPER`. Every crate Cargo compiles goes
through the real compiler; after analysis the driver gets the `TyCtxt` and extracts:

- **definitions** with their kind, the span of their name, owner (the enclosing item), canonical path and
  signature;
- **references**: every resolved path, method call, field access, macro call and attribute, with the
  `DefId` it resolves to. Method calls and fields come from rustc's `TypeckResults`, so `x.len()` points at
  the impl that rustc selected, through auto-deref and trait resolution, not at a guess by name;
- **types** of definitions, fields, generic parameters, `where` predicates, impl headers and associated
  type bindings, lowered from `rustc_middle::ty::Ty` into a structural, hash-consed type arena;
- **macros** (`macro_rules!` bodies), `use` items, active `cfg`s, lang items, and short **notes** — texts
  the compiler knows about a span and hover shows as they are: the generic arguments of a call
  (`T` = `String`) and the memory layout of a type, field or binding (`size = 8, align = 0x8, no Drop`).

Documentation comments are not copied into the index; hover reads them from the source file (or
`rust-src` for the standard library) at the definition's span. The driver then lets compilation continue
normally, so the `cargo check` result is unchanged.

### Dependencies are indexed once per machine

A dependency does not change once Cargo has downloaded it. Its shard goes to a content-addressed store,
`~/.cache/ridx/v5/objects/`, under a unit key built from the crate's name and version, Cargo's
disambiguator (`-C extra-filename`), its manifest directory, the target triple, its `--cfg` flags, the
**content digests** of the `--extern` artifacts it was compiled against, the contents of its build-script
output directory, the format and extractor versions and the driver's own hash; a crate outside the registry
also adds the hash of the workspace's `Cargo.lock`. If a valid shard with that key exists, the driver
skips extraction (rustc itself still runs whenever Cargo rebuilds the crate; on later checks Cargo skips
unchanged dependencies anyway). Two projects that use the same `serde` from the same Cargo home with the
same features share one shard; a second worktree of the same project indexes only its own crates.

The standard library (`core`, `alloc`, `std`) is indexed from its metadata into
`~/.cache/ridx/v5/sysroot/<key>/`, once per toolchain, so navigation and hover reach into `std` without
`ridx` analyzing its sources.

### Workspace crates and variants

The workspace's own crates are written under `target/ridx/variants/<id>/` with a manifest that lists every
shard the workspace needs, its own and its dependencies'. The variant id is a hash of what changes the
compiled code: profile, target, package selection (`-p`), features (`--features`, `--all-features`,
`--no-default-features`), the `Cargo.toml` files, `Cargo.lock`, `.cargo/config.toml`, the rustc version,
the toolchain, `RUSTFLAGS` and `CARGO_ENCODED_RUSTFLAGS`, and the format and extractor versions. Switching
features or targets produces another variant next to the first instead of overwriting it.

### A failed build keeps the last good index

When `cargo check` fails, the crates it did rebuild replace their shards and every crate it could not
build keeps its previous one, so a syntax error in one file does not empty the index of a project that
was indexed before. (A first build that fails before any crate was indexed reports the error.)

### Reindexing after a save

The server starts `ridx index` in the background after each save and after a source file changes on disk
(a checkout, a pull), one run per workspace at a time; changes made during a run are folded into one more
run. When only workspace sources changed, a fast path calls the driver directly on the changed crates and
their dependents in topological order, with the arguments Cargo used last time, without starting Cargo.
A dependent is compiled only when the interface of a crate it depends on changed: the crate's tokens
without comments, without the bodies of ordinary functions and without positions
(`direct/interface.rs`). When only positions moved (a body edit, lines added above a definition), the
references in the dependent's shard are moved to the new positions of their definitions in place
(`ridx_format::relocate`). A dependent left uncompiled keeps metadata built against the crate's older
build, so it is compiled before anything that is compiled against both (`direct/cutoff.rs`).
It falls back to Cargo when the build configuration changed, a new `.rs` file appeared, the previous check
had failed crates, a changed target is a `proc-macro`, or more than ten packages are affected. The server
watches the manifest and swaps in the new index when it is published.

## 2. The `.ridx` shard

One shard per compiled crate. Format version 5, little-endian, with fixed-size records in sections whose
offsets are aligned to 8 bytes, so every table is an array of `#[repr(C)]` structs that can be read in
place:

| Section | Record | Used for |
|---|---|---|
| header | 416 B | version, crate, unit key, target, features, section offsets |
| string pool | — | names, paths and signatures, referenced by offset |
| files | 16 B | source files with content hashes (an open buffer is matched to the text that was compiled) |
| definitions | 40 B | with an FNV-1a hash index by path and an index by owner |
| deref | 16 B | `Deref` targets: completion after `.` and receiver chains |
| references | 48 B | sorted by span (what is under the cursor), with a hash index by target (find references) |
| types | 28 B | the type at a span, sorted by span |
| type arena | — | structural types of definitions, fields and signatures |
| impls, assoc types | 32 B, 16 B | trait and inherent impls, `type Item = …` bindings |
| fields, generics, where | 20 B, 20 B, 28 B | struct and variant fields, generic parameters, bounds and supertraits |
| uses, macros, cfgs, lang items, notes | 28 B, 12 B, 8 B, 12 B, 28 B | auto-import, macro expansion for live code, hover texts |

The reader validates the header and maps the file; the file descriptor is closed once it is mapped. A
lookup in a shard is a hash probe, a binary search over a sorted table, or a scan of a table in mapped
memory; the records themselves are never decoded into heap structures. Pages that are never touched are
never read from disk, and the kernel can drop the ones that were under memory pressure and read them again
later. This is why a `ridx` process on Bevy stays under 1 GB while the index for the workspace and all its
dependencies is on disk.

`WorkspaceIndex` opens the manifest, maps every shard it names (the workspace's, its dependencies' and the
standard library's), and builds the few indices that span crates in memory: which unit a file belongs to,
which crates can see which (for references across crates and re-exports), definitions by position, paths,
the standard-library prelude, and the mapping of `std::` paths to their canonical `core::` and `alloc::`
definitions.

## 3. Answering a request

For most requests the server finds what is under the cursor in the reference table of the file's shard and
follows its target. The main sources per request:

| Request | Answer from |
|---|---|
| definition, declaration | the reference's target definition; across crates through the target's canonical path |
| references, highlight, rename | the target hash index of every shard that can see the defining crate |
| implementation, type hierarchy | the impl table and supertraits, with impl headers read from source as a fallback |
| hover | the definition, its type and notes from the index; doc comments from the source |
| type definition, inlay hints | the types table and type arena, the live layer, parameter names from the callee's source |
| call hierarchy | incoming: references that are calls or uses as a value, grouped by the enclosing function; outgoing: calls in the function's body, resolved through the index |
| completion | the live layer's type of the receiver, then the owner and deref tables; symbol search and `use` items for auto-import |
| signature help | the callee's definition and signature, its parameter names and docs from source |
| diagnostics | `cargo check` on save (flycheck), merged with the live layer's own checks |
| code actions | assists with compile-safety preconditions checked against the index and the source |
| formatting | `rustfmt` |

The driver's answers are rustc's own, so on code that has been built a definition or a type is what the
compiler decided, including through macros, `impl Trait`, trait objects and auto-deref.

Compiler diagnostics are reported against the saved file. Zed moves them over unsaved edits itself, so
it receives them as saved; other editors receive them mapped onto the open buffer.

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
  (`RidxV5ItemDb`) that loads each crate's tables when it is first needed, so the live layer never
  analyzes a whole crate.

When the next index arrives, each buffer is matched to it through the recorded hash, and the overlay
starts over from the new compiled text.

## 5. Trade-offs

- **The index is built by `cargo check`.** The first build of a large workspace takes minutes; after that
  only changed crates are rebuilt, and dependencies already in the store are not extracted again.
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

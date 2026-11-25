# SCIP range encodings

Four SCIP indexes of one definition each, written by the `scip` crate 0.10.0 and read by `ridx compare`'s loader
(`crates/ridx/src/compare/scip_loader.rs`). The test `test_scip_range_fixtures_load_as_their_encoding_says` in
`crates/ridx/tests/e2e/scip_loader.rs` builds each one and checks that the file here is that index, byte for byte.
Every definition is in the document `src/lib.rs`; positions are 0-based.

| File | `Occurrence` | The loader reads |
|---|---|---|
| `old-single-line.scip` | `range = [1, 4, 9]` | line 1, characters 4 to 9 |
| `old-multi-line.scip` | `range = [3, 2, 5, 1]` (start line, start, end line, end) | line 3 from character 2 to the line's end |
| `typed-only.scip` | `range` empty, `single_line_range {line 7, start_character 4, end_character 10}` | line 7, characters 4 to 10 |
| `both-typed-wins.scip` | `range = [20, 0, 3]` and `single_line_range {14, 6, 8}` | line 14, characters 6 to 8: the typed range wins |

The typed `single_line_range` / `multi_line_range` oneof came with scip-code/scip PR 387; the `repeated int32 range`
field is deprecated since. `RIDX_BLESS_SCIP_FIXTURES=1 cargo test -p ridx --test e2e scip_loader` writes the files
again.

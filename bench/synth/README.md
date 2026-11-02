# Synthetic LSP Workspace Generator & Ground-Truth Oracle (`v1`)

`bench/synth/` generates deterministic, seed-varying multi-crate Rust workspaces (`core_pkg`, `service_pkg`, `app_pkg`, plus seed-dependent auxiliary crates and submodule files) where every definition, reference, implementation, fold, and rename target has known 0-based UTF-16 ground-truth coordinates in `ground_truth.json`.

## Key `v1` Capabilities

1. **Seed-Varying Workspace Topology (`workspace_gen.py`, `workspace_gen_ext.py`)**:
   - Varying crate count (`4` crates for even seeds, `5` crates for odd seeds), file-backed submodules (`crates/core_pkg/src/sub_mod_<tag>.rs`), inline module nesting depths (`1..3`), and declaration permutations.
   - Verified automatically on every run via `verify_seed_variation()` (`seed=42` vs `seed=7`).
2. **Rename Oracle `v1` (`126` cases at default `--scale 6`)**:
   - Covers all 13 refactoring families:
     1. Struct field (`value_<tag>`)
     2. Struct field init shorthand (`Foo { x }` -> `Foo { x_renamed: x }` when renaming field `x`, and `Foo { y: y_renamed }` when renaming local `y`)
     3. Trait method across concrete impls, `&T` reference impls, and blanket impls
     4. `pub use` re-exports across 3+ crates
     5. `use ... as` alias (renaming alias vs renaming original symbol before `as`)
     6. Module rename with file rename (`RenameFile` in `documentChanges`)
     7. Symbol inside macro callsite (`apply_op!(macro_target(x))`)
     8. `Self` keyword inside `impl` block
     9. Local variable across expressions
     10. Generic type parameter `T`
     11. Lifetime parameter `'a`
     12. Associated type and associated const (`<Carrier as AssocSpec>::OutputTy` / `MAX_LIMIT`)
     13. Enum variant (`State::Ready`)
   - Covers 3 refusal categories (`expect_error: True`): Rust keyword cursor (`fn`), external trait impl item (`core::fmt::Display::fmt`), and invalid identifier (`123invalid_ident`).
   - **Post-edit `cargo check` gate**: Every positive `rename` edit (including `RenameFile`) is applied to the workspace and verified with `cargo check --workspace --tests --quiet` before restoring the original files.
3. **Exact Set `Precision` & `Recall`**:
   - Computes exact set `precision` and `recall` across `definition`, `declaration`, `type_definition`, `implementation`, `references`, `document_highlight`, `rename`, `prepare_rename`, `linked_editing_range`, `incoming_calls`, `outgoing_calls`, `supertypes`, `subtypes`, `document_symbol`, `folding_range`, and `completion` (expected receiver members + forbidden out-of-scope symbols).
4. **RA vs Oracle Divergence Report (`ra_divergences`)**:
   - When `--ra rust-analyzer` is supplied, records every case where `ridx` satisfies the ground-truth oracle and passes `cargo check` while `rust-analyzer` diverges.

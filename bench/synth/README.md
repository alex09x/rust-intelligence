# Synthetic LSP Workspace Generator & Oracle (`v0` Scaffold)

> **Status: `v0` Scaffold.**
> This directory initially provides the `v0` structural scaffold (`source_builder.py`, `workspace_gen.py`, `oracle_cases.py`, `oracle_eval.py`, `run_synth_oracle.py`) for generating deterministic multi-crate Cargo workspaces, indexing them via `RUSTC_WORKSPACE_WRAPPER`, and querying LSP servers (`ridx` and `rust-analyzer`).
>
> In `v0`:
> - The generated workspace topology is fixed across seeds (only identifier names vary with `--seed`).
> - Several evaluators check unilateral presence (`expected_name in labels`, `issubset`, substring match) rather than exact set equality, precision/recall, or post-rename `cargo check` compilation.
>
> Do not use `v0` scores as a proof of "100% correctness". Full exact-set verification, seed-varying module/crate topologies, all 13 `rename` families + refusal cases, and post-edit `cargo check` compilation verification are tracked in `v1`.

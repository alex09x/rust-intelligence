# bench/

Benchmark tooling for `ridx` against `rust-analyzer`. Everything here is Python 3 with no
third-party dependencies except `pytest` for the harness tests. Methodology:
`BENCHMARK_METHODOLOGY.md`. Published data: `published/` (tracked); raw runs: `results/`
(ignored).

## Live pipeline: the 78-method LSP matrix

```
python3 bench/matrix_runner.py --repo <corpus dir> --ridx-bin target/release/ridx \
    --ra-bin rust-analyzer --output bench/results/matrix_<repo>_<node>.json
python3 bench/publish_results.py --src bench/results --pub bench/published
python3 bench/matrix_report.py            # docs/BENCHMARK_MATRIX.md from published matrix runs
python3 bench/export_site.py              # site data from published runs
```

| File | Role |
|---|---|
| `matrix_runner.py` | Entry point: samples the matrix, runs both servers, writes the result JSON with provenance. |
| `matrix_execute.py` | Runs the 78 methods against one server: lifecycle, sync, queries (cold then two warm asks), mutations; a/b/c classification. |
| `matrix_sampling.py`, `scan.py` | Deterministic sampling (seed 42) of 300 items per method from a regex scan of the corpus sources. |
| `matrix_categories.py` | The 78 method specs and the request parameters built for each sampled item. |
| `matrix_scenarios.py` | Synchronization notifications and the three overlay mutation cases. |
| `matrix_canary.py`, `lsp_barrier.py` | rust-analyzer readiness (progress tokens plus definition canary) and the drain barrier between methods. |
| `lsp_process.py`, `lsp_capabilities.py` | Framed JSON-RPC client with a reader thread, cancel on timeout, the advertised client capabilities. |
| `matrix_provenance.py`, `provenance.py` | Index precondition, ridx/driver/sysroot/repo provenance recorded into every run. |
| `metrics.py` | Percentiles and process RSS/fd/thread telemetry. |
| `publish_results.py`, `publish_common.py` | Path sanitising and provenance sidecars for `published/`. |
| `matrix_report.py` | Renders `docs/BENCHMARK_MATRIX.md` from the published runs only. |
| `export_site.py` | Exports published numbers for the website. |
| `selection_range_fidelity.py` | `textDocument/selectionRange` chains of one or more ridx binaries against rust-analyzer on a corpus (methodology, section 9). |
| `test_*.py` | Harness unit tests: `python3 -m pytest bench`. |

## Superseded by the matrix, kept because published data and the comparison report cite them

- `pilot_runner.py`, `worktree_bench.py`, `corpus.py`, `sampling.py`, `report.py`: the five-repository
  pilot (`published/pilot_5repos_2026-10-08.jsonl`) and `LSP_BENCHMARK_COMPARISON_REPORT.md`.
- `run_completion_benchmark.py`, `verify_completion_regression.py`, `validate_scenarios.py`: the
  completion recall/precision reruns (`published/rerun_completion_*.jsonl`,
  `published/completion_regression_*.json`).

Remove them together with a report and site regenerated from the matrix data, not before.

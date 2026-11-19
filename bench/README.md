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
python3 bench/comparison_report.py        # LSP_BENCHMARK_COMPARISON_REPORT.md from published matrix runs
python3 bench/export_site.py              # bench.json and bench.data.js from published matrix runs (no longer on the site)
python3 bench/measure_resources.py ...    # RSS and CPU of one server under an editor session (Linux)
python3 bench/export_resources.py bench/published/resources_<commit>.jsonl <commit> <date>   # site/resources.data.js
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
| `matrix_report.py` | Renders `docs/BENCHMARK_MATRIX.md` (all 78 methods per repository, provenance, known harness defects) from the published runs only. |
| `comparison_report.py` | Renders `LSP_BENCHMARK_COMPARISON_REPORT.md` (setup, resources, headline methods, coverage differences, earlier publications) from the published runs only. |
| `matrix_headline.py` | Headline method list, latency formatting and the coverage sets shared by the report and the site export. |
| `export_site.py` | Builds `bench.json` and `bench.data.js` from the published matrix runs (the site showed them until `d13fbd23`). |
| `measure_resources.py`, `export_resources.py` | Peak memory, CPU and start-up of ridx and rust-analyzer under the same editor session; the site's numbers (`site/resources.data.js`) are exported from the published run and none is typed by hand. |
| `selection_range_fidelity.py` | `textDocument/selectionRange` chains of one or more ridx binaries against rust-analyzer on a corpus (methodology, section 9). |
| `position_ab.py`, `position_ab_classify.py` | Per-position A/B of the matrix requests between ridx binaries and, optionally, rust-analyzer: outcome by the matrix rule, answer digest, target kind of the file and token kind under the cursor per position (methodology, section 10). |
| `test_*.py` | Harness unit tests: `python3 -m pytest bench`. |

## Earlier pipelines

Their published data stays in `published/`; the tooling was removed from the tree after commit
`9a75047`, the last commit that still contains it. Restore the files from that commit and run them
from the current tree:

- Five-repository pilot (`published/pilot_5repos_2026-10-08.jsonl`):
  `git checkout 9a75047 -- bench/pilot_runner.py bench/worktree_bench.py bench/corpus.py bench/sampling.py bench/report.py bench/metrics.py bench/corpus`,
  then `python3 bench/pilot_runner.py --ridx target/release/ridx --ra rust-analyzer --taskset 24-31 --samples 300`
  and `python3 bench/report.py --results bench/published/pilot_5repos_2026-10-08.jsonl --output LSP_BENCHMARK_COMPARISON_REPORT.md`
  for the pilot version of the comparison report.
- Completion recall and precision reruns (`published/rerun_completion_*.jsonl`,
  `published/completion_regression_*.json`):
  `git checkout 9a75047 -- bench/run_completion_benchmark.py bench/verify_completion_regression.py bench/validate_scenarios.py bench/metrics.py`,
  then `python3 bench/run_completion_benchmark.py --commit <ridx sha> --ridx-bin <ridx> --ra-bin <rust-analyzer>` and
  `python3 bench/verify_completion_regression.py --base-commit <sha> --curr-commit <sha> --replay`.

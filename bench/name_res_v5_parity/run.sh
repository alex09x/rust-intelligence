#!/usr/bin/env bash
# rust-intelligence — Phase 2.5 7-corpus reference parity & benchmark runner
# Copyright (c) 2026 Alexander Panasenko
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

missing=()
[[ -z "${RUSTC_BIN:-}" ]] && missing+=("RUSTC_BIN")
[[ -z "${RIDX_DEPS_DIR:-}" ]] && missing+=("RIDX_DEPS_DIR")
[[ -z "${RIDX_FORMAT_RLIB:-}" ]] && missing+=("RIDX_FORMAT_RLIB")
if (( ${#missing[@]} > 0 )); then
  echo "error: missing required environment variable(s): ${missing[*]}" >&2
  echo "usage: RUSTC_BIN=/path/to/rustc RIDX_DEPS_DIR=/path/to/deps RIDX_FORMAT_RLIB=/path/to/libridx_format.rlib $0 [args...]" >&2
  exit 2
fi

if [[ ! -x "$RUSTC_BIN" ]]; then
  echo "error: RUSTC_BIN is not executable: $RUSTC_BIN" >&2
  exit 2
fi
if [[ ! -d "$RIDX_DEPS_DIR" ]]; then
  echo "error: RIDX_DEPS_DIR is not a directory: $RIDX_DEPS_DIR" >&2
  exit 2
fi
if [[ ! -f "$RIDX_FORMAT_RLIB" ]]; then
  echo "error: RIDX_FORMAT_RLIB does not exist: $RIDX_FORMAT_RLIB" >&2
  exit 2
fi

OUT_BIN_DIR="${OUT_BIN_DIR:-${TMPDIR:-/tmp}/ridx-name-res-v5-parity-bin}"
mkdir -p "$OUT_BIN_DIR"

SERDE_JSON_RLIB="$(ls -t "$RIDX_DEPS_DIR"/libserde_json-*.rlib | head -n1)"
MEMMAP2_RLIB="$(ls -t "$RIDX_DEPS_DIR"/libmemmap2-*.rlib | head -n1)"

"$RUSTC_BIN" --edition 2021 -O -D warnings \
  --crate-name ridx_syntax --crate-type rlib \
  "$REPO_ROOT/crates/ridx-syntax/src/lib.rs" \
  -o "$OUT_BIN_DIR/libridx_syntax.rlib"

"$RUSTC_BIN" --edition 2021 -O -D warnings \
  --crate-name name_res_v5_parity \
  "$SCRIPT_DIR/main.rs" \
  --extern "ridx_syntax=$OUT_BIN_DIR/libridx_syntax.rlib" \
  --extern "ridx_format=$RIDX_FORMAT_RLIB" \
  --extern "serde_json=$SERDE_JSON_RLIB" \
  --extern "memmap2=$MEMMAP2_RLIB" \
  -L "dependency=$RIDX_DEPS_DIR" \
  -o "$OUT_BIN_DIR/name_res_v5_parity"

exec timeout "${PARITY_TIMEOUT:-600s}" "$OUT_BIN_DIR/name_res_v5_parity" "$@"

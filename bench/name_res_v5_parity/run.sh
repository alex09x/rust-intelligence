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

RUSTC_BIN="${RUSTC_BIN:-/home/alex09x/.rustup/toolchains/1.98.1-x86_64-unknown-linux-gnu/bin/rustc}"
if [[ ! -x "$RUSTC_BIN" ]]; then
  RUSTC_BIN="rustc"
fi

RIDX_DEPS_DIR="${RIDX_DEPS_DIR:-/tmp/ridx-b9-ws/target/release/deps}"
RIDX_FORMAT_RLIB="${RIDX_FORMAT_RLIB:-/tmp/ridx-b9-ws/target/release/libridx_format.rlib}"
OUT_BIN_DIR="${OUT_BIN_DIR:-/tmp/ridx-v5-2_5/bin}"
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

exec timeout "${PARITY_TIMEOUT:-120s}" "$OUT_BIN_DIR/name_res_v5_parity" "$@"

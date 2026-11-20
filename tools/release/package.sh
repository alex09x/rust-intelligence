#!/usr/bin/env bash
# rust-intelligence — package ridx and ridx-driver of one target as ridx-<target>.tar.gz with its .sha256.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# usage: package.sh BIN_DIR TARGET OUT_DIR   (run from the repository root)
set -euo pipefail
bin=$1 target=$2 out=$3
name="ridx-$target"
mkdir -p "$out/$name"
cp "$bin/ridx" "$bin/ridx-driver" LICENSE-MIT LICENSE-APACHE README.md "$out/$name/"
chmod 0755 "$out/$name/ridx" "$out/$name/ridx-driver"
tar -C "$out" -czf "$out/$name.tar.gz" "$name"
if command -v sha256sum >/dev/null; then sum="sha256sum"; else sum="shasum -a 256"; fi
(cd "$out" && $sum "$name.tar.gz" > "$name.tar.gz.sha256")
cat "$out/$name.tar.gz.sha256"

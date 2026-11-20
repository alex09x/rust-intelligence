#!/usr/bin/env bash
# rust-intelligence — smoke test of release binaries: index a small crate and resolve a method call.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# usage: smoke.sh BIN_DIR   (BIN_DIR holds ridx and ridx-driver)
set -euo pipefail
bin=$(cd "$1" && pwd)
work=$(mktemp -d "${RUNNER_TEMP:-${TMPDIR:-/tmp}}/ridx-smoke.XXXXXX")
mkdir -p "$work/src"
printf '[package]\nname = "smoke"\nversion = "0.1.0"\nedition = "2021"\n\n[dependencies]\n' > "$work/Cargo.toml"
printf 'pub struct Greeter;\nimpl Greeter {\n    pub fn hello(&self) -> String { String::from("hi") }\n}\npub fn run() -> String { Greeter.hello() }\n' > "$work/src/lib.rs"
cd "$work"
"$bin/ridx" --version
"$bin/ridx" index .
out=$("$bin/ridx" def . src/lib.rs:5:35)
echo "$out"
echo "$out" | grep -q 'lib.rs:3'

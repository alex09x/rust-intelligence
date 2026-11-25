/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "node:test";
import { expectedSha, findRidx, isOlder, parseVersion, releaseArchive, releaseTarget } from "../src/install.ts";

test("a release target for each platform the release builds, a reason for the rest", () => {
  assert.equal(releaseTarget("darwin", "arm64").target, "aarch64-apple-darwin");
  assert.equal(releaseTarget("linux", "x64").target, "x86_64-unknown-linux-gnu");
  assert.equal(releaseTarget("linux", "arm64").target, "aarch64-unknown-linux-gnu");
  assert.equal(releaseTarget("darwin", "x64").target, "x86_64-apple-darwin");
  assert.equal(releaseTarget("win32", "x64").target, "x86_64-pc-windows-msvc");
  assert.match(releaseTarget("win32", "arm64").why ?? "", /win32 arm64/);
  assert.equal(releaseArchive("x86_64-pc-windows-msvc"), "ridx-x86_64-pc-windows-msvc.zip");
  assert.equal(releaseArchive("x86_64-apple-darwin"), "ridx-x86_64-apple-darwin.tar.gz");
});

test("the digest of a .sha256 file", () => {
  assert.equal(expectedSha("FE56a3  ridx-aarch64-apple-darwin.tar.gz\n"), "fe56a3");
});

test("the configured path wins, then the extension's own install", () => {
  const dir = mkdtempSync(join(tmpdir(), "ridx-find-"));
  assert.equal(findRidx(join(dir, "missing"), dir), undefined, "a configured path that does not exist is not replaced");
  mkdirSync(join(dir, "bin"));
  writeFileSync(join(dir, "bin", "ridx"), "");
  assert.equal(findRidx(undefined, dir), join(dir, "bin", "ridx"));
  writeFileSync(join(dir, "mine"), "");
  assert.equal(findRidx(join(dir, "mine"), dir), join(dir, "mine"));
});

test("an installed ridx older than the latest release", () => {
  const installed = parseVersion("ridx 0.3.6 (commit: a13d9d72, dirty: false)");
  assert.deepEqual(installed, [0, 3, 6]);
  assert.equal(isOlder(installed!, parseVersion("v0.3.13")!), true, "0.3.6 < 0.3.13, not compared as text");
  assert.equal(isOlder([0, 3, 13], [0, 3, 13]), false);
  assert.equal(isOlder([0, 4, 0], [0, 3, 13]), false);
  assert.equal(parseVersion("version unknown"), undefined);
});

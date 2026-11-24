/*
 * rust-intelligence — Ultra-fast, zero-memory code intelligence for Rust
 * Copyright (c) 2026 Alexander Panasenko
 *
 * Contact: alex@prod.codes
 * Author: https://prod.codes/about/
 * Project: https://github.com/alex09x/rust-intelligence
 * SPDX-License-Identifier: MIT OR Apache-2.0
 */

// Finding `ridx`, and installing the release binaries the way site/install.sh does: the archive for this platform,
// checked against its SHA-256, plus the Rust toolchain ridx-driver links to.

import { execFile } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync } from "node:fs";
import { chmod, copyFile, mkdir, mkdtemp, rename, rm, writeFile } from "node:fs/promises";
import { homedir, tmpdir } from "node:os";
import { delimiter, join } from "node:path";
import { promisify } from "node:util";

const run = promisify(execFile);

export const REPO = "alex09x/rust-intelligence";
/** The compiler ridx-driver links to; the same as site/install.sh. The user's projects keep their own toolchain. */
export const TOOLCHAIN = "1.98.1";

/** The release target of this machine, or why there is no prebuilt binary for it. */
export function releaseTarget(platform = process.platform, arch = process.arch): { target?: string; why?: string } {
  if (platform === "linux" && arch === "x64") return { target: "x86_64-unknown-linux-gnu" };
  if (platform === "linux" && arch === "arm64") return { target: "aarch64-unknown-linux-gnu" };
  if (platform === "darwin" && arch === "arm64") return { target: "aarch64-apple-darwin" };
  if (platform === "darwin") return { why: "there is no prebuilt ridx for Intel Macs" };
  return { why: `there is no prebuilt ridx for ${platform} ${arch}` };
}

/** `ridx` from the setting, the extension's own install, the PATH, ~/.cargo/bin or ~/.local/bin, in that order. */
export function findRidx(configured: string | undefined, storageDir: string): string | undefined {
  if (configured) return existsSync(configured) ? configured : undefined;
  const exe = process.platform === "win32" ? "ridx.exe" : "ridx";
  const dirs = [join(storageDir, "bin"), ...(process.env.PATH ?? "").split(delimiter).filter((d) => d !== "")];
  dirs.push(join(homedir(), ".cargo", "bin"), join(homedir(), ".local", "bin"));
  return dirs.map((d) => join(d, exe)).find((p) => existsSync(p));
}

/** The first field of a `.sha256` file: `<hex>  <name>`. */
export function expectedSha(text: string): string {
  return text.trim().split(/\s+/)[0].toLowerCase();
}

async function download(url: string): Promise<Buffer> {
  const response = await fetch(url, { redirect: "follow" });
  if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
  return Buffer.from(await response.arrayBuffer());
}

/**
 * Installs `ridx` and `ridx-driver` of the latest release into `<storageDir>/bin` and returns the path of `ridx`.
 * `say` reports each step. Throws with the reason when a step fails; nothing is left half-installed.
 */
export async function installRidx(storageDir: string, say: (step: string) => void): Promise<string> {
  const { target, why } = releaseTarget();
  if (!target) throw new Error(`${why}: install it with \`cargo install ridx ridx-driver --locked\``);
  say(`installing Rust ${TOOLCHAIN} for ridx-driver`);
  try {
    await run("rustup", ["toolchain", "install", TOOLCHAIN, "--profile", "minimal", "--component", "rust-src"]);
  } catch (e) {
    throw new Error(`rustup failed (is it installed? https://rustup.rs): ${(e as Error).message}`);
  }
  const archive = `ridx-${target}.tar.gz`;
  const base = `https://github.com/${REPO}/releases/latest/download`;
  say(`downloading ${archive}`);
  const [data, sums] = await Promise.all([download(`${base}/${archive}`), download(`${base}/${archive}.sha256`)]);
  const got = createHash("sha256").update(data).digest("hex");
  if (got !== expectedSha(sums.toString("utf8"))) throw new Error(`checksum mismatch for ${archive}`);

  const tmp = await mkdtemp(join(tmpdir(), "ridx-"));
  try {
    await writeFile(join(tmp, archive), data);
    await run("tar", ["-xzf", join(tmp, archive), "-C", tmp]);
    const bin = join(storageDir, "bin");
    await mkdir(bin, { recursive: true });
    // ridx finds ridx-driver next to itself.
    for (const name of ["ridx-driver", "ridx"]) {
      // Copied next to the old binary, then renamed over it: the tmp dir may be on another volume.
      await copyFile(join(tmp, `ridx-${target}`, name), join(bin, `${name}.new`));
      await chmod(join(bin, `${name}.new`), 0o755);
      await rename(join(bin, `${name}.new`), join(bin, name));
    }
    return join(bin, "ridx");
  } finally {
    await rm(tmp, { recursive: true, force: true });
  }
}

/** `[major, minor, patch]` of `ridx 0.3.6 (commit: …)` or `v0.3.13`, or undefined. */
export function parseVersion(text: string): number[] | undefined {
  const m = /(\d+)\.(\d+)\.(\d+)/.exec(text);
  return m ? [Number(m[1]), Number(m[2]), Number(m[3])] : undefined;
}

/** Whether version `a` comes before `b`. */
export function isOlder(a: number[], b: number[]): boolean {
  for (let i = 0; i < 3; i++) if (a[i] !== b[i]) return a[i] < b[i];
  return false;
}

/** The tag of the latest release (`v0.3.13`), or undefined when GitHub does not answer. */
export async function latestRelease(): Promise<string | undefined> {
  try {
    const response = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, { headers: { Accept: "application/vnd.github+json" } });
    if (!response.ok) return undefined;
    return ((await response.json()) as { tag_name?: string }).tag_name;
  } catch {
    return undefined;
  }
}

/** `ridx --version`, or undefined when it does not run. */
export async function ridxVersion(ridx: string): Promise<string | undefined> {
  try {
    return (await run(ridx, ["--version"])).stdout.trim();
  } catch {
    return undefined;
  }
}

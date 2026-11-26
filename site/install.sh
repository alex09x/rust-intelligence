#!/bin/sh
# rust-intelligence — installer for ridx and ridx-driver release binaries
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# curl -fsSL https://rust-intelligence.com/install.sh | sh
#
# Installs the Rust toolchain ridx-driver runs on (1.98.1, through rustup, with rust-src for the standard
# library sources), then the release archive for this platform into ~/.local/bin after checking its SHA-256.
#   RIDX_VERSION=v0.3.7   a release tag instead of the latest release
#   RIDX_INSTALL_DIR=DIR  where the two binaries go (default ~/.local/bin)
set -eu

REPO="alex09x/rust-intelligence"
TOOLCHAIN="1.98.1"
INSTALL_DIR="${RIDX_INSTALL_DIR:-$HOME/.local/bin}"

say() { printf 'ridx: %s\n' "$*"; }
die() { printf 'ridx: %s\n' "$*" >&2; exit 1; }

case "$(uname -s)-$(uname -m)" in
  Linux-x86_64) target=x86_64-unknown-linux-gnu ;;
  Linux-aarch64 | Linux-arm64) target=aarch64-unknown-linux-gnu ;;
  Darwin-arm64) target=aarch64-apple-darwin ;;
  Darwin-x86_64) target=x86_64-apple-darwin ;;
  MINGW* | MSYS* | CYGWIN*) die "on Windows, run in PowerShell: irm https://rust-intelligence.com/install.ps1 | iex" ;;
  *) die "unsupported platform $(uname -s) $(uname -m); prebuilt binaries cover Linux, macOS and Windows (x86_64): https://github.com/$REPO#install" ;;
esac

command -v rustup >/dev/null 2>&1 || die "rustup is required: https://rustup.rs, then run this again"
command -v curl >/dev/null 2>&1 || die "curl is required"
if command -v sha256sum >/dev/null 2>&1; then sha() { sha256sum "$1" | cut -d' ' -f1; }
else sha() { shasum -a 256 "$1" | cut -d' ' -f1; }; fi

say "installing Rust $TOOLCHAIN (the compiler ridx-driver links to; your projects keep their own toolchain)"
rustup toolchain install "$TOOLCHAIN" --profile minimal --component rust-src >/dev/null

if [ -n "${RIDX_VERSION:-}" ]; then
  base="https://github.com/$REPO/releases/download/$RIDX_VERSION"
else
  base="https://github.com/$REPO/releases/latest/download"
fi
archive="ridx-$target.tar.gz"
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

say "downloading $archive"
curl -fsSL "$base/$archive" -o "$tmp/$archive" || die "download failed: $base/$archive"
curl -fsSL "$base/$archive.sha256" -o "$tmp/$archive.sha256" || die "download failed: $base/$archive.sha256"
want=$(cut -d' ' -f1 "$tmp/$archive.sha256")
got=$(sha "$tmp/$archive")
[ "$want" = "$got" ] || die "checksum mismatch for $archive: expected $want, got $got"

tar -xzf "$tmp/$archive" -C "$tmp"
mkdir -p "$INSTALL_DIR"
for bin in ridx ridx-driver; do
  install -m 0755 "$tmp/ridx-$target/$bin" "$INSTALL_DIR/$bin.new"
  mv -f "$INSTALL_DIR/$bin.new" "$INSTALL_DIR/$bin"
done
say "installed $("$INSTALL_DIR/ridx" --version) into $INSTALL_DIR"

case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *)
    case "${SHELL:-}" in
      */zsh) rc="$HOME/.zshrc" ;;
      */bash) if [ "$(uname -s)" = Darwin ]; then rc="$HOME/.bash_profile"; else rc="$HOME/.bashrc"; fi ;;
      */fish) rc="" ;;
      *) rc="$HOME/.profile" ;;
    esac
    if [ -n "$rc" ]; then
      say "add $INSTALL_DIR to PATH: echo 'export PATH=\"$INSTALL_DIR:\$PATH\"' >> $rc, then open a new terminal"
    else
      say "add $INSTALL_DIR to PATH: fish_add_path $INSTALL_DIR"
    fi
    ;;
esac
say "next: point your editor at ridx — https://rust-intelligence.com/#start"

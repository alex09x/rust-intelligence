# rust-intelligence — Windows installer for ridx and ridx-driver release binaries
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# powershell -ExecutionPolicy Bypass -c "irm https://rust-intelligence.com/install.ps1 | iex"
#
# Installs the Rust toolchain ridx-driver runs on (1.98.1, through rustup, with rust-src for the standard
# library sources), then the release archive for this machine into %USERPROFILE%\.local\bin after checking its
# SHA-256, and adds that directory to the user's PATH.
#   $env:RIDX_VERSION = "v0.3.14"   a release tag instead of the latest release
#   $env:RIDX_INSTALL_DIR = "DIR"   where the two binaries go
$ErrorActionPreference = "Stop"
$Repo = "alex09x/rust-intelligence"
$Toolchain = "1.98.1"
$InstallDir = if ($env:RIDX_INSTALL_DIR) { $env:RIDX_INSTALL_DIR } else { Join-Path $env:USERPROFILE ".local\bin" }

function Say($m) { Write-Host "ridx: $m" }
function Die($m) { Write-Host "ridx: $m" -ForegroundColor Red; exit 1 }

if ([System.Environment]::Is64BitOperatingSystem -ne $true -or $env:PROCESSOR_ARCHITECTURE -ne "AMD64") {
    Die "a prebuilt binary exists for Windows on x86_64 only; build from source: https://github.com/$Repo#install"
}
$Target = "x86_64-pc-windows-msvc"
if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) { Die "rustup is required: https://rustup.rs, then run this again" }

Say "installing Rust $Toolchain (the compiler ridx-driver links to; your projects keep their own toolchain)"
rustup toolchain install $Toolchain --profile minimal --component rust-src | Out-Null
if ($LASTEXITCODE -ne 0) { Die "rustup toolchain install $Toolchain failed" }

$Base = if ($env:RIDX_VERSION) { "https://github.com/$Repo/releases/download/$($env:RIDX_VERSION)" } else { "https://github.com/$Repo/releases/latest/download" }
$Archive = "ridx-$Target.zip"
$Tmp = Join-Path ([System.IO.Path]::GetTempPath()) ("ridx-install-" + [System.Guid]::NewGuid())
New-Item -ItemType Directory -Path $Tmp | Out-Null
try {
    Say "downloading $Archive"
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Archive" -OutFile (Join-Path $Tmp $Archive)
    Invoke-WebRequest -UseBasicParsing -Uri "$Base/$Archive.sha256" -OutFile (Join-Path $Tmp "$Archive.sha256")
    $Want = ((Get-Content (Join-Path $Tmp "$Archive.sha256") -Raw).Trim() -split '\s+')[0].ToLower()
    $Got = (Get-FileHash -Algorithm SHA256 (Join-Path $Tmp $Archive)).Hash.ToLower()
    if ($Want -ne $Got) { Die "checksum mismatch for ${Archive}: expected $Want, got $Got" }

    Expand-Archive -Path (Join-Path $Tmp $Archive) -DestinationPath $Tmp -Force
    New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
    foreach ($Bin in "ridx.exe", "ridx-driver.exe") {
        Copy-Item -Force (Join-Path $Tmp "ridx-$Target\$Bin") (Join-Path $InstallDir $Bin)
    }
} finally {
    Remove-Item -Recurse -Force $Tmp -ErrorAction SilentlyContinue
}
Say "installed $(& (Join-Path $InstallDir 'ridx.exe') --version) into $InstallDir"

$UserPath = [System.Environment]::GetEnvironmentVariable("Path", "User")
if (-not (($UserPath -split ';') -contains $InstallDir)) {
    [System.Environment]::SetEnvironmentVariable("Path", (@($InstallDir) + ($UserPath -split ';' | Where-Object { $_ })) -join ';', "User")
    Say "added $InstallDir to your PATH; open a new terminal"
}
Say "next: point your editor at ridx: https://rust-intelligence.com/#start"

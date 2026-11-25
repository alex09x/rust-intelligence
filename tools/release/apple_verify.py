# rust-intelligence — verify the signed macOS binaries without any signing secret.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# usage: apple_verify.py SIGNED_DIR EVIDENCE_DIR
# Checks for each binary: the Developer ID of APPLE_TEAM_ID, hardened runtime, a secure timestamp, the
# entitlements ridx-driver needs and no others, Apple's accepted ticket for this CDHash, and Apple's
# online notarization assessment.
import argparse
import json
import plistlib
import re
import time
from pathlib import Path

from apple_keychain import expected_team, run

BINARIES = ("ridx", "ridx-driver")
IDENTIFIERS = {"ridx": "com.rust-intelligence.ridx", "ridx-driver": "com.rust-intelligence.ridx-driver"}
DRIVER_ENTITLEMENTS = {"com.apple.security.cs.disable-library-validation": True, "com.apple.security.cs.allow-dyld-environment-variables": True}


def signature(binary, team):
    requirement = f'anchor apple generic and certificate leaf[subject.OU] = "{team}" and certificate leaf[field.1.2.840.113635.100.6.1.13] exists'
    run(["codesign", "--verify", "--strict", "-R", "=" + requirement, binary])
    detail = run(["codesign", "--display", "--verbose=4", binary]).stderr
    if "Signature=adhoc" in detail or f"TeamIdentifier={team}" not in detail:
        raise ValueError(f"{binary.name}: Developer ID team/signature mismatch")
    flags = re.search(r"flags=0x([0-9a-fA-F]+)", detail)
    cdhash = re.search(r"^CDHash=([0-9a-fA-F]{40}|[0-9a-fA-F]{64})$", detail, re.MULTILINE)
    if not flags or not int(flags[1], 16) & 0x10000 or "Timestamp=" not in detail or not cdhash:
        raise ValueError(f"{binary.name}: hardened runtime, secure timestamp and CDHash are mandatory")
    return cdhash[1].lower()


def entitlements(binary):
    out = run(["codesign", "--display", "--entitlements", "-", "--xml", binary]).stdout.strip()
    return plistlib.loads(out.encode()) if out else {}


def check_entitlements(name, actual):
    expected = DRIVER_ENTITLEMENTS if name == "ridx-driver" else {}
    if actual != expected:
        raise ValueError(f"{name}: entitlements {sorted(actual)} differ from the expected {sorted(expected)}")


def accepted_ticket(receipt, job, name, cdhash, arch):
    if receipt.get("status") != "Accepted" or receipt.get("jobId") != job or receipt.get("issues") not in (None, []):
        raise ValueError("Apple did not accept this exact submission")
    if not any(t.get("cdhash", "").lower() == cdhash and t.get("arch") == arch and t.get("path", "").endswith("ridx/" + name) for t in receipt.get("ticketContents", [])):
        raise ValueError(f"Apple's ticket does not cover the final {name} ({arch})")


def architecture(binary):
    """The binary's one architecture as Apple's ticket names it (arm64, x86_64)."""
    return run(["lipo", "-archs", binary]).stdout.strip()


def verify(signed, evidence):
    team = expected_team()
    job = json.loads((evidence / "submission.json").read_text())["id"]
    receipt = json.loads((evidence / "apple-notary-log.json").read_text())
    for name in BINARIES:
        binary = signed / name
        cdhash = signature(binary, team)
        check_entitlements(name, entitlements(binary))
        accepted_ticket(receipt, job, name, cdhash, architecture(binary))
        for attempt in range(6):
            if run(["codesign", "--verify", "--strict", "-R=notarized", "--check-notarization", binary], timeout=90, check=False).returncode == 0:
                break
            if attempt == 5:
                raise ValueError(f"{name}: Apple's online notarization assessment did not pass")
            time.sleep(10)
        print("verified", name, "team", team, "cdhash", cdhash)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("signed", type=Path)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    verify(args.signed, args.evidence)

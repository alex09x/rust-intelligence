# rust-intelligence — Developer ID signing and notarization of the macOS release binaries.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# usage: apple_sign.py UNSIGNED_DIR SIGNED_DIR EVIDENCE_DIR
# Copies ridx and ridx-driver from UNSIGNED_DIR, signs them with hardened runtime and a secure timestamp
# (ridx-driver with tools/release/ridx-driver.entitlements), submits both to Apple and fails unless the
# submission is Accepted and Apple's ticket covers each final binary. The binaries are never executed here,
# while the keys are loaded; apple_verify.py runs them in a job without secrets.
import argparse
import hashlib
import json
import shutil
import tempfile
import zipfile
from pathlib import Path

from apple_keychain import credentials, expected_team, run
from apple_verify import BINARIES, IDENTIFIERS, accepted_ticket, signature

ENTITLEMENTS = Path(__file__).resolve().parent / "ridx-driver.entitlements"


def sign_command(auth, name, binary):
    command = ["codesign", "--force", "--sign", auth["identity"], "--keychain", auth["keychain"], "--identifier", IDENTIFIERS[name], "--options", "runtime", "--timestamp"]
    if name == "ridx-driver":
        command += ["--entitlements", ENTITLEMENTS]
    return command + [binary]


def sign_release(source, output, evidence):
    output.mkdir(parents=True, exist_ok=False)
    evidence.mkdir(parents=True, exist_ok=False)
    team = expected_team()
    for name in BINARIES:
        shutil.copy2(source / name, output / name)
        (output / name).chmod(0o755)
    with tempfile.TemporaryDirectory(prefix="ridx-notary-payload-") as directory:
        submission_zip = Path(directory) / "submission.zip"
        with credentials() as auth:
            cdhashes = {}
            for name in BINARIES:
                run(sign_command(auth, name, output / name))
                cdhashes[name] = signature(output / name, team)
            # Exactly the two signed binaries; the private P12/P8/Keychain live in another directory.
            with zipfile.ZipFile(submission_zip, "w", zipfile.ZIP_DEFLATED) as archive:
                for name in BINARIES:
                    archive.write(output / name, "ridx/" + name)
            result = run(["xcrun", "notarytool", "submit", submission_zip, *auth["auth"], "--wait", "--timeout", "20m", "--output-format", "json"], timeout=1300)
            submission = json.loads(result.stdout)
            job = submission.get("id", "")
            log_file = evidence / "apple-notary-log.json"
            if job:
                run(["xcrun", "notarytool", "log", job, *auth["auth"], log_file], timeout=120, check=False)
            if submission.get("status") != "Accepted":
                raise ValueError(f"Apple notarization was {submission.get('status')!r}, not Accepted (see apple-notary-log.json)")
        receipt = json.loads(log_file.read_text())
        zip_sha = hashlib.sha256(submission_zip.read_bytes()).hexdigest()
        if receipt.get("sha256", "").lower() != zip_sha:
            raise ValueError("Apple's receipt does not describe the submitted ZIP")
    for name in BINARIES:
        accepted_ticket(receipt, job, name, cdhashes[name])
    (evidence / "submission.json").write_text(json.dumps({"id": job, "status": "Accepted", "zip_sha256": zip_sha, "cdhash": cdhashes, "team": team}, indent=2) + "\n")
    print("ridx and ridx-driver signed with Developer ID and notarized: Accepted", job)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("source", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("evidence", type=Path)
    args = parser.parse_args()
    sign_release(args.source, args.output, args.evidence)

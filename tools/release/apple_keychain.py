# rust-intelligence — Apple release credentials in a temporary Keychain on a hosted macOS runner.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# The same credential model as the release of Mitts Build: six secrets of the `release-signing`
# Environment and the repository variable APPLE_TEAM_ID. Nothing private is written to the log.
import argparse
import base64
import contextlib
import json
import os
import re
import secrets
import shlex
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path

REPOSITORY = "alex09x/rust-intelligence"
PREFIX = "ridx-apple-private-"
STATE = "ridx-apple-keychain-state.json"
SECRET_NAMES = (
    "APPLE_CERTIFICATE_P12_BASE64",
    "APPLE_CERTIFICATE_PASSWORD",
    "APPLE_SIGNING_IDENTITY",
    "APPLE_NOTARY_KEY_P8",
    "APPLE_NOTARY_KEY_ID",
    "APPLE_NOTARY_ISSUER_ID",
)
_redactions = []


def redact(value):
    for secret in _redactions:
        if secret:
            value = value.replace(secret, "[REDACTED]")
    return value


def run(args, timeout=120, check=True):
    try:
        result = subprocess.run([str(a) for a in args], capture_output=True, text=True, timeout=timeout, check=False)
    except subprocess.TimeoutExpired:
        raise RuntimeError(f"{Path(str(args[0])).name} timed out; private arguments suppressed") from None
    if check and result.returncode:
        raise RuntimeError(f"{Path(str(args[0])).name} failed ({result.returncode}): " + redact(result.stderr + result.stdout)[-6000:])
    return result


def release_ref(ref):
    """Keys are only for this repository's release tags and its main branch (a manual rerun)."""
    return ref == "refs/heads/main" or re.fullmatch(r"refs/tags/v\d+\.\d+\.\d+", ref or "") is not None


def hosted_release():
    if sys.platform != "darwin" or os.environ.get("GITHUB_ACTIONS") != "true" or os.environ.get("GITHUB_REPOSITORY") != REPOSITORY or not release_ref(os.environ.get("GITHUB_REF", "")):
        raise ValueError("Apple private keys are restricted to this repository's hosted release jobs")


def expected_team():
    value = os.environ.get("APPLE_TEAM_ID", "")
    if not re.fullmatch(r"[A-Z0-9]{10}", value):
        raise ValueError("APPLE_TEAM_ID must identify the expected Developer ID team")
    return value


@contextlib.contextmanager
def credentials():
    hosted_release()
    values = {name: os.environ.get(name, "") for name in SECRET_NAMES}
    if any(not value for value in values.values()):
        raise ValueError("Required Apple release secrets are missing; no ad-hoc fallback")
    _redactions.extend(values.values())
    identity = values["APPLE_SIGNING_IDENTITY"].upper()
    if not re.fullmatch(r"[A-F0-9]{40}", identity):
        raise ValueError("Expected one exact Developer ID certificate fingerprint")
    parent = Path(os.environ["RUNNER_TEMP"]).resolve()
    original = shlex.split(run(["security", "list-keychains", "-d", "user"]).stdout)
    with tempfile.TemporaryDirectory(prefix=PREFIX, dir=parent) as directory:
        root = Path(directory)
        root.chmod(0o700)
        keychain = root / "signing.keychain-db"
        p12 = root / "identity.p12"
        api_key = root / "notary.p8"
        password = secrets.token_urlsafe(48)
        _redactions.append(password)
        state = parent / STATE
        state.write_text(json.dumps({"root": str(root), "keychain": str(keychain), "search": original}))
        try:
            p12.write_bytes(base64.b64decode(values["APPLE_CERTIFICATE_P12_BASE64"], validate=True))
            api_key.write_text(values["APPLE_NOTARY_KEY_P8"])
            p12.chmod(0o600)
            api_key.chmod(0o600)
            run(["security", "create-keychain", "-p", password, keychain])
            run(["security", "set-keychain-settings", "-lut", "3600", keychain])
            run(["security", "unlock-keychain", "-p", password, keychain])
            run(["security", "import", p12, "-k", keychain, "-P", values["APPLE_CERTIFICATE_PASSWORD"], "-T", "/usr/bin/codesign"])
            p12.unlink()
            run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", password, keychain])
            # codesign looks the private key up through the user search list: add the temporary Keychain
            # in front of the runner's own entries and restore them in `finally`.
            run(["security", "list-keychains", "-d", "user", "-s", keychain, *original])
            searchable = shlex.split(run(["security", "list-keychains", "-d", "user"]).stdout)
            if str(keychain) not in searchable or any(path not in searchable for path in original):
                raise ValueError("Temporary signing Keychain is missing from the user search list")
            identities = run(["security", "find-identity", "-v", "-p", "codesigning", keychain]).stdout
            found = re.findall(r'\b([A-F0-9]{40})\s+"Developer ID Application:[^"\n]+"', identities)
            if found != [identity] or f"({expected_team()})" not in identities:
                raise ValueError("Temporary Keychain must contain exactly the selected valid Developer ID")
            yield {
                "identity": identity,
                "keychain": keychain,
                "auth": ["--key", str(api_key), "--key-id", values["APPLE_NOTARY_KEY_ID"], "--issuer", values["APPLE_NOTARY_ISSUER_ID"]],
            }
        finally:
            run(["security", "delete-keychain", keychain], check=False)
            run(["security", "list-keychains", "-d", "user", "-s", *original], check=False)
            state.unlink(missing_ok=True)


def cleanup():
    parent = Path(os.environ["RUNNER_TEMP"]).resolve()
    state = parent / STATE
    if not state.is_file():
        return
    value = json.loads(state.read_text())
    root = Path(value["root"]).resolve()
    keychain = Path(value["keychain"]).resolve()
    if root.parent != parent or not root.name.startswith(PREFIX) or keychain.parent != root:
        raise ValueError("Refusing cleanup outside the owned Apple credential directory")
    run(["security", "delete-keychain", keychain], check=False)
    run(["security", "list-keychains", "-d", "user", "-s", *value["search"]], check=False)
    shutil.rmtree(root, ignore_errors=False)
    state.unlink()


def check_credentials():
    with credentials() as auth:
        run(["xcrun", "notarytool", "history", *auth["auth"], "--output-format", "json"], timeout=60)
    print("Apple identity import and Notary API authentication passed; nothing was signed or notarized")


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("operation", choices=["check", "cleanup"], nargs="?", default="check")
    args = parser.parse_args()
    cleanup() if args.operation == "cleanup" else check_credentials()

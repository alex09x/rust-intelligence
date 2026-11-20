# rust-intelligence — the owner's one-time upload of the Apple release credentials to GitHub.
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# usage (on the Mac whose login Keychain holds the Developer ID Application identity):
#   python3 tools/release/provision_apple_secrets.py --p8 PATH/AuthKey_XXXX.p8 --issuer ISSUER_UUID [--key-id XXXX]
#
# Exports only the Developer ID Application identity of APPLE_TEAM_ID (macOS asks to allow the export),
# re-encrypts it with a fresh random password in a format `security import` reads on the runner, and sets
# the six secrets of the `release-signing` Environment with `gh secret set` over stdin. Nothing is printed,
# written to disk or passed on a command line.
import argparse
import base64
import re
import secrets
import subprocess
import sys
from pathlib import Path

REPO = "alex09x/rust-intelligence"
ENVIRONMENT = "release-signing"
TEAM = "284V2M3LN9"


def run(args, data=None):
    result = subprocess.run(args, input=data, capture_output=True, check=False)
    if result.returncode:
        sys.exit(f"{args[0]} {args[1] if len(args) > 1 else ''} failed ({result.returncode}): {result.stderr.decode(errors='replace')[-800:]}")
    return result.stdout


def developer_id():
    listing = run(["security", "find-identity", "-v", "-p", "codesigning"]).decode()
    found = re.findall(rf'\b([A-F0-9]{{40}})\s+"(Developer ID Application:[^"\n]+\({TEAM}\))"', listing)
    if len(found) != 1:
        sys.exit(f"expected exactly one Developer ID Application identity of team {TEAM}, found {len(found)}")
    return found[0]


def pem_blocks(pem):
    """(bag attributes, PEM block) pairs of `openssl pkcs12 -nodes` output."""
    return re.findall(r"(Bag Attributes.*?)(-----BEGIN ([A-Z ]+)-----.*?-----END \3-----\n?)", pem, re.S)


def select_identity(pem, name):
    blocks = pem_blocks(pem)
    certs = [(attrs, block) for attrs, block, kind in blocks if kind == "CERTIFICATE" and name in attrs]
    if len(certs) != 1:
        sys.exit("the exported Keychain data has no single certificate of the Developer ID identity")
    key_id = re.search(r"localKeyID:\s*([0-9A-F ]+)", certs[0][0])
    keys = [block for attrs, block, kind in blocks if kind.endswith("PRIVATE KEY") and key_id and key_id[1].strip() in attrs]
    if len(keys) != 1:
        sys.exit("the private key of the Developer ID certificate was not found in the export")
    return keys[0] + certs[0][1]


def p12_of_developer_id(name):
    transfer = secrets.token_urlsafe(32)
    # A transfer password that lives only in this process; openssl gets it through its environment.
    exported = run(["security", "export", "-t", "identities", "-f", "pkcs12", "-P", transfer])
    pem = subprocess.run(["openssl", "pkcs12", "-nodes", "-passin", "env:RIDX_P12_TRANSFER"], input=exported, capture_output=True, env={"RIDX_P12_TRANSFER": transfer, "PATH": "/usr/bin:/bin:/opt/homebrew/bin"}, check=False)
    if pem.returncode:
        sys.exit("openssl could not read the Keychain export: " + pem.stderr.decode(errors="replace")[-800:])
    selected = select_identity(pem.stdout.decode(), name)
    password = secrets.token_urlsafe(32)
    # 3DES/SHA-1 PKCS#12 is what `security import` on macOS runners reads reliably.
    out = subprocess.run(["openssl", "pkcs12", "-export", "-passout", "env:RIDX_P12_OUT", "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1"], input=selected.encode(), capture_output=True, env={"RIDX_P12_OUT": password, "PATH": "/usr/bin:/bin:/opt/homebrew/bin"}, check=False)
    if out.returncode:
        sys.exit("openssl could not build the PKCS#12: " + out.stderr.decode(errors="replace")[-800:])
    return base64.b64encode(out.stdout).decode(), password


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--p8", type=Path, required=True, help="App Store Connect API key file AuthKey_<KEY_ID>.p8")
    parser.add_argument("--issuer", required=True, help="App Store Connect issuer ID (UUID)")
    parser.add_argument("--key-id", help="key ID; taken from the file name AuthKey_<KEY_ID>.p8 by default")
    args = parser.parse_args()
    from_name = re.fullmatch(r"AuthKey_([A-Z0-9]+)\.p8", args.p8.name)
    key_id = args.key_id or (from_name[1] if from_name else None)
    if not key_id or not re.fullmatch(r"[A-Z0-9]{10}", key_id):
        sys.exit("pass --key-id: the 10-character App Store Connect key ID")
    if not re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}", args.issuer.lower()):
        sys.exit("--issuer must be the issuer UUID from App Store Connect, Users and Access, Integrations")
    p8 = args.p8.read_text()
    if "BEGIN PRIVATE KEY" not in p8:
        sys.exit(f"{args.p8} is not an App Store Connect private key")
    fingerprint, name = developer_id()
    print(f"identity: {name}; macOS will ask to allow exporting it from the login Keychain")
    p12, password = p12_of_developer_id(name)
    values = {
        "APPLE_CERTIFICATE_P12_BASE64": p12,
        "APPLE_CERTIFICATE_PASSWORD": password,
        "APPLE_SIGNING_IDENTITY": fingerprint,
        "APPLE_NOTARY_KEY_P8": p8,
        "APPLE_NOTARY_KEY_ID": key_id,
        "APPLE_NOTARY_ISSUER_ID": args.issuer.lower(),
    }
    for secret, value in values.items():
        run(["gh", "secret", "set", secret, "--repo", REPO, "--env", ENVIRONMENT], value.encode())
        print("set", secret)
    print(f"done: {len(values)} secrets of {REPO} / {ENVIRONMENT}. Check with the Apple credentials workflow.")


if __name__ == "__main__":
    main()

# rust-intelligence — tests of the macOS signing and verification rules (no keys, no network).
# Copyright (c) 2026 Alexander Panasenko
#
# Contact: alex@prod.codes
# Author: https://prod.codes/about/
# Project: https://github.com/alex09x/rust-intelligence
# SPDX-License-Identifier: MIT OR Apache-2.0
#
# python3 tools/release/test_apple_release.py   (the codesign test runs on macOS only)
import plistlib
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import apple_keychain  # noqa: E402
import apple_sign  # noqa: E402
import apple_verify  # noqa: E402


class ReleaseRules(unittest.TestCase):
    def test_keys_only_for_release_tags_and_main(self):
        self.assertTrue(apple_keychain.release_ref("refs/tags/v0.3.8"))
        self.assertTrue(apple_keychain.release_ref("refs/heads/main"))
        for ref in ("refs/heads/feature", "refs/tags/v0.3.8-rc1", "refs/pull/1/merge", ""):
            self.assertFalse(apple_keychain.release_ref(ref), ref)

    def test_only_the_driver_gets_entitlements(self):
        auth = {"identity": "A" * 40, "keychain": "k"}
        self.assertNotIn("--entitlements", apple_sign.sign_command(auth, "ridx", Path("ridx")))
        driver = apple_sign.sign_command(auth, "ridx-driver", Path("ridx-driver"))
        self.assertEqual(driver[driver.index("--entitlements") + 1], apple_sign.ENTITLEMENTS)
        self.assertIn("runtime", driver)
        self.assertIn("--timestamp", driver)

    def test_entitlements_file_is_exactly_what_verify_expects(self):
        self.assertEqual(plistlib.loads(apple_sign.ENTITLEMENTS.read_bytes()), apple_verify.DRIVER_ENTITLEMENTS)
        apple_verify.check_entitlements("ridx", {})
        apple_verify.check_entitlements("ridx-driver", dict(apple_verify.DRIVER_ENTITLEMENTS))
        with self.assertRaises(ValueError):
            apple_verify.check_entitlements("ridx", dict(apple_verify.DRIVER_ENTITLEMENTS))
        with self.assertRaises(ValueError):
            apple_verify.check_entitlements("ridx-driver", {"com.apple.security.cs.disable-library-validation": True})

    def test_ticket_must_cover_each_binary(self):
        receipt = {"status": "Accepted", "jobId": "j", "issues": None, "ticketContents": [
            {"path": "submission.zip/ridx/ridx", "arch": "arm64", "cdhash": "AA"},
            {"path": "submission.zip/ridx/ridx-driver", "arch": "arm64", "cdhash": "BB"},
        ]}
        apple_verify.accepted_ticket(receipt, "j", "ridx", "aa")
        apple_verify.accepted_ticket(receipt, "j", "ridx-driver", "bb")
        with self.assertRaises(ValueError):
            apple_verify.accepted_ticket(receipt, "j", "ridx-driver", "aa")
        with self.assertRaises(ValueError):
            apple_verify.accepted_ticket(receipt, "other", "ridx", "aa")
        with self.assertRaises(ValueError):
            apple_verify.accepted_ticket(dict(receipt, status="Invalid"), "j", "ridx", "aa")

    @unittest.skipUnless(sys.platform == "darwin" and shutil.which("codesign"), "needs codesign")
    def test_codesign_accepts_the_entitlements_file(self):
        with tempfile.TemporaryDirectory() as directory:
            binary = Path(directory) / "true"
            shutil.copyfile("/usr/bin/true", binary)
            binary.chmod(0o755)
            subprocess.run(["codesign", "--remove-signature", binary], check=False, capture_output=True)
            subprocess.run(["codesign", "-f", "-s", "-", "-o", "runtime", "--entitlements", apple_sign.ENTITLEMENTS, binary], check=True, capture_output=True)
            self.assertEqual(apple_verify.entitlements(binary), apple_verify.DRIVER_ENTITLEMENTS)


@unittest.skipUnless(sys.platform == "darwin" and shutil.which("openssl"), "needs macOS security and openssl")
class ProvisionIdentity(unittest.TestCase):
    """The Developer ID identity is picked out of a multi-identity export and re-encrypted for `security import`."""

    def identity(self, directory, cn, name):
        key, cert = Path(directory) / (name + ".key"), Path(directory) / (name + ".crt")
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", cert, "-days", "2", "-subj", "/CN=" + cn], check=True, capture_output=True)
        return key.read_text() + cert.read_text()

    def test_select_and_reexport(self):
        import provision_apple_secrets as provision

        name = "Developer ID Application: Test (284V2M3LN9)"
        with tempfile.TemporaryDirectory() as directory:
            pems = [self.identity(directory, "Apple Development: Test", "dev"), self.identity(directory, name, "devid")]
            p12s = []
            for i, pem in enumerate(pems):
                friendly = name if i else "Apple Development: Test"
                out = subprocess.run(["openssl", "pkcs12", "-export", "-name", friendly, "-passout", "pass:t", "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1"], input=pem.encode(), capture_output=True, check=True).stdout
                p12s.append(subprocess.run(["openssl", "pkcs12", "-nodes", "-passin", "pass:t"], input=out, capture_output=True, check=True).stdout.decode())
            selected = provision.select_identity("".join(p12s), name)
            self.assertEqual(selected.count("PRIVATE KEY-----") // 2, 1)
            self.assertIn(pems[1].split("-----BEGIN CERTIFICATE-----")[1].split("-----END")[0].strip()[:64], selected)
            out = subprocess.run(["openssl", "pkcs12", "-export", "-passout", "pass:p", "-certpbe", "PBE-SHA1-3DES", "-keypbe", "PBE-SHA1-3DES", "-macalg", "sha1"], input=selected.encode(), capture_output=True, check=True).stdout
            p12 = Path(directory) / "selected.p12"
            p12.write_bytes(out)
            keychain = Path(directory) / "t.keychain-db"
            subprocess.run(["security", "create-keychain", "-p", "k", keychain], check=True, capture_output=True)
            try:
                imported = subprocess.run(["security", "import", p12, "-k", keychain, "-P", "p"], capture_output=True, text=True)
                self.assertEqual(imported.returncode, 0, imported.stderr)
                self.assertIn("1 identity imported", imported.stdout)
            finally:
                subprocess.run(["security", "delete-keychain", keychain], capture_output=True)


if __name__ == "__main__":
    unittest.main()

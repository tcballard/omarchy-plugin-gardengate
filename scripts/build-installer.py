#!/usr/bin/env python3
"""Build a self-contained Linux installer from a clean, committed checkout."""
import base64
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import zlib

root = Path(__file__).resolve().parents[1]
output = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "dist"


def run(*args):
    return subprocess.check_output(args, cwd=root, text=True).strip()


if run("git", "status", "--porcelain", "--untracked-files=all"):
    sys.exit("Commit the source first; installers require a clean checkout.")
source = run("git", "rev-parse", "HEAD")
version = json.loads((root / "manifest.json").read_text())["version"]
subprocess.run(["cargo", "build", "--locked", "--release", "--manifest-path", "companion/Cargo.toml"],
               cwd=root, check=True)
binary = root / "companion/target/release/gardengate"
if f"gardengate {version} " not in run(str(binary), "--version"):
    sys.exit("Manifest and binary versions disagree.")
installer_source = (root / "scripts/prebuilt_installer.py").read_text()
files = {
    "gardengate": binary.read_bytes(),
    "desktop": (root / "companion/packaging/io.github.tcballard.gardengate.desktop").read_bytes(),
    "watch-service": (root / "companion/packaging/gardengate.service").read_bytes(),
    "pull-service": (root / "companion/packaging/gardengate-pull.service").read_bytes(),
    "license": (root / "LICENSE").read_bytes(),
    "notices": (root / "companion/THIRD-PARTY-NOTICES.md").read_bytes(),
    "uninstaller": installer_source.encode(),
}
with tempfile.TemporaryDirectory() as temp:
    bundle = Path(temp) / "source.bundle"
    run("git", "bundle", "create", str(bundle), "HEAD")
    files["source-bundle"] = bundle.read_bytes()
payload = {"version": version, "source": source, "rustc": run("rustc", "--version"), "files": {}}
for name, data in files.items():
    payload["files"][name] = {"sha256": hashlib.sha256(data).hexdigest(), "size": len(data),
                              "data": base64.b64encode(zlib.compress(data, 9)).decode()}
script = installer_source.replace("PAYLOAD = {}", "PAYLOAD = " + repr(payload), 1)
header = '''#!/usr/bin/env bash
set -euo pipefail
command -v python3 >/dev/null || { echo 'Python 3 is required (included with Omarchy).' >&2; exit 1; }
# Keep stdin attached to the caller for sudo/pacman confirmation prompts.
exec python3 /dev/fd/3 "$@" 3<<'GARDENGATE_PYTHON'
'''
output.mkdir(parents=True, exist_ok=True)
target = output / f"gardengate-{version}-linux-x86_64.run"
target.write_text(header + script + "\nGARDENGATE_PYTHON\n")
target.chmod(0o755)
checksum = hashlib.sha256(target.read_bytes()).hexdigest()
(output / "SHA256SUMS").write_text(f"{checksum}  {target.name}\n")
for item in payload["files"].values():
    del item["data"]
(output / "build-manifest.json").write_text(json.dumps(payload, indent=2) + "\n")
print(f"Built {target}\nSHA-256 {checksum}\nSource {source}")

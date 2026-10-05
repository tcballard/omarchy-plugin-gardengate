"""Per-user installer embedded by build-installer.py; standard library only."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import shutil
import subprocess
import sys
import tempfile
import zlib

PLUGIN = "io.github.tcballard.gardengate"
REPOSITORY = "https://github.com/tcballard/omarchy-plugin-gardengate.git"
RECORD = ".local/share/gardengate/installer/files.json"
PAYLOAD = {}  # Replaced with a compressed, source-bound payload by the builder.


def run(*args, **kwargs):
    return subprocess.run(args, check=True, **kwargs)


def digest(data):
    return hashlib.sha256(data).hexdigest()


def safe_path(home, relative):
    parts = Path(relative).parts
    if Path(relative).is_absolute() or not parts or ".." in parts:
        raise RuntimeError(f"Unsafe install path: {relative}")
    target = home
    for part in parts:
        target = target / part
        if target.is_symlink():
            raise RuntimeError(f"Refusing symlink: {target}")
    return target


def atomic_write(path, data, mode):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=".gardengate-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as output:
            output.write(data)
            output.flush()
            os.fsync(output.fileno())
        os.chmod(name, mode)
        os.replace(name, path)
    finally:
        if os.path.exists(name):
            os.unlink(name)


def read_record(home):
    path = safe_path(home, RECORD)
    if not path.exists():
        return {}
    result = json.loads(path.read_text())
    if result.get("schema") != 1 or not isinstance(result.get("files"), dict):
        raise RuntimeError("Unknown installer record; leaving files unchanged.")
    return result["files"]


def preflight_files(home, files):
    previous = read_record(home)
    # Check every destination before the first write. Never adopt unmanaged files.
    for relative in set(files) | set(previous):
        path = safe_path(home, relative)
        if path.exists():
            if not path.is_file() or relative not in previous:
                raise RuntimeError(f"Unmanaged destination; left unchanged: {path}")
            if digest(path.read_bytes()) != previous[relative]:
                raise RuntimeError(f"Modified installed file; left unchanged: {path}")
    return previous


def install_files(home, files):
    previous = preflight_files(home, files)
    records = {}
    for relative, (data, mode) in files.items():
        atomic_write(safe_path(home, relative), data, mode)
        records[relative] = digest(data)
    for relative in previous.keys() - files.keys():
        safe_path(home, relative).unlink(missing_ok=True)
    atomic_write(safe_path(home, RECORD), json.dumps({
        "schema": 1, "source": PAYLOAD.get("source"), "files": records
    }, indent=2).encode() + b"\n", 0o600)


def uninstall(home):
    previous = read_record(home)
    if not previous:
        raise RuntimeError("No prebuilt-install record found; nothing removed.")
    # Refuse the whole removal before stopping services if any managed file changed.
    preflight_files(home, {})
    run("systemctl", "--user", "disable", "--now", "gardengate.service")
    run("systemctl", "--user", "stop", "gardengate-pull.service")
    for relative in previous:
        safe_path(home, relative).unlink(missing_ok=True)
    safe_path(home, RECORD).unlink()
    run("systemctl", "--user", "daemon-reload")
    print("Companion removed. Inbox, credentials, recovery copies and bar plugin retained.")
    print(f"To remove the bar plugin: omarchy plugin remove {PLUGIN}")


def dependencies():
    required = {"rclone": "rclone", "secret-tool": "libsecret", "kdialog": "kdialog",
                "git": "git", "xdg-open": "xdg-utils"}
    missing = [package for command, package in required.items() if not shutil.which(command)]
    if missing:
        print("Installing runtime packages: " + " ".join(missing), flush=True)
        run("sudo", "pacman", "-S", "--needed", *missing)
    output = run("rclone", "version", capture_output=True, text=True).stdout
    match = re.search(r"rclone v(\d+)\.(\d+)\.(\d+)", output)
    if not match or tuple(map(int, match.groups())) < (1, 75, 1):
        raise RuntimeError("rclone 1.75.1+ is required. Update Omarchy, then rerun this installer.")
    run("systemctl", "--user", "show-environment", stdout=subprocess.DEVNULL)


def decode_payload():
    files = {}
    for name, item in PAYLOAD["files"].items():
        data = zlib.decompress(base64.b64decode(item["data"], validate=True))
        if len(data) != item["size"] or digest(data) != item["sha256"]:
            raise RuntimeError(f"Damaged installer payload: {name}")
        files[name] = data
    return files


def install(home, skip_plugin):
    payload = decode_payload()
    # Smoke-test the shipped executable before changing the installation.
    with tempfile.TemporaryDirectory(prefix="gardengate-check-") as temp:
        binary = Path(temp) / "gardengate"
        binary.write_bytes(payload["gardengate"])
        binary.chmod(0o700)
        run(str(binary), "--version")
    if any(char in str(home) for char in ('"', '%', '\\', '`', '$', '\n', '\r')):
        raise RuntimeError("Home path cannot be safely represented in the desktop launcher.")
    desktop = payload["desktop"].decode().replace(
        "Exec=gardengate manage", f'Exec="{home}/.local/bin/gardengate" manage')
    files = {
        ".local/bin/gardengate": (payload["gardengate"], 0o755),
        ".config/systemd/user/gardengate.service": (payload["watch-service"], 0o644),
        ".config/systemd/user/gardengate-pull.service": (payload["pull-service"], 0o644),
        f".local/share/applications/{PLUGIN}.desktop": (desktop.encode(), 0o644),
        ".local/share/gardengate/installer/uninstall.py": (payload["uninstaller"], 0o644),
        ".local/share/gardengate/installer/LICENSE": (payload["license"], 0o644),
        ".local/share/gardengate/installer/THIRD-PARTY-NOTICES.md": (payload["notices"], 0o644),
    }
    preflight_files(home, files)
    dependencies()
    # Replacing a running executable must not leave the old watcher running.
    for service in ("gardengate.service", "gardengate-pull.service"):
        active = subprocess.run(["systemctl", "--user", "is-active", "--quiet", service])
        if active.returncode == 0:
            raise RuntimeError(f"Stop {service} before updating: systemctl --user stop {service}")
        if active.returncode not in (3, 4):
            raise RuntimeError(f"Could not inspect {service}; nothing installed.")
    install_files(home, files)
    run("systemctl", "--user", "daemon-reload")
    print("Prebuilt companion installed. No Apple login or downloads have been started.", flush=True)
    if not skip_plugin:
        plugin_path = home / ".config/omarchy/plugins" / PLUGIN
        if plugin_path.exists() or plugin_path.is_symlink():
            print("Existing bar plugin left unchanged. Use Omarchy's plugin manager to update it.")
        else:
            with tempfile.TemporaryDirectory(prefix="gardengate-plugin-") as temp:
                bundle = Path(temp) / "source.bundle"
                bundle.write_bytes(payload["source-bundle"])
                checkout = Path(temp) / "plugin"
                run("git", "clone", "--quiet", str(bundle), str(checkout))
                run("git", "-C", str(checkout), "checkout", "--quiet", "--detach", PAYLOAD["source"])
                run("omarchy", "plugin", "add", str(checkout), "--enable", "--yes")
                installed = run("git", "-C", str(plugin_path), "rev-parse", "HEAD",
                                capture_output=True, text=True).stdout.strip()
                if installed != PAYLOAD["source"]:
                    raise RuntimeError("Unexpected installed plugin revision; origin left unchanged.")
                run("git", "-C", str(plugin_path), "remote", "set-url", "origin", REPOSITORY)
                run("git", "-C", str(plugin_path), "checkout", "--quiet", "-B", "main")
                run("git", "-C", str(plugin_path), "config", "branch.main.remote", "origin")
                run("git", "-C", str(plugin_path), "config", "branch.main.merge", "refs/heads/main")
    print("Next: ~/.local/bin/gardengate connect")
    print("Uninstall: python3 ~/.local/share/gardengate/installer/uninstall.py --uninstall")


def main():
    parser = argparse.ArgumentParser(description="Garden Gate prebuilt developer-preview installer")
    parser.add_argument("--check", action="store_true", help="Verify embedded payload without installing")
    parser.add_argument("--no-plugin", action="store_true", help="Install only the companion")
    parser.add_argument("--uninstall", action="store_true", help="Remove unchanged managed companion files")
    args = parser.parse_args()
    if args.check:
        decode_payload()
        print(f"Payload verified: Garden Gate {PAYLOAD['version']}, source {PAYLOAD['source']}")
        return
    if os.geteuid() == 0:
        raise RuntimeError("Run as your normal desktop user, without sudo.")
    if platform.system() != "Linux" or platform.machine() != "x86_64":
        raise RuntimeError("This installer supports x86_64 Linux only.")
    home = Path.home()
    if args.uninstall:
        uninstall(home)
    else:
        if not shutil.which("omarchy") and not args.no_plugin:
            raise RuntimeError("Omarchy is required to install the bar plugin.")
        install(home, args.no_plugin)


if __name__ == "__main__":
    try:
        main()
    except (RuntimeError, OSError, ValueError, subprocess.CalledProcessError) as error:
        print(f"Garden Gate: {error}", file=sys.stderr)
        sys.exit(1)

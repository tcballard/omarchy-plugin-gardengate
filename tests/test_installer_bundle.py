#!/usr/bin/env python3
"""Exercise the real .run installer with real Git/binary and stub desktop services."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile

artifact = Path(sys.argv[1]).resolve()
# Some build sandboxes have only uid 0 and cannot create a second user. In that
# case execute the exact embedded Python with only geteuid simulated; every
# destination still lives under the fixture HOME and desktop commands are stubs.
simulated_user = os.geteuid() == 0


with tempfile.TemporaryDirectory(prefix="gardengate-install-test-") as temp:
    home = Path(temp) / "home"
    home.mkdir()
    stubs = Path(temp) / "stubs"
    stubs.mkdir()
    scripts = {
        "rclone": "#!/bin/sh\necho 'rclone v1.75.1'\n",
        "systemctl": "#!/bin/sh\ncase \"$*\" in *is-active*) exit 3;; esac\nexit 0\n",
        "omarchy": '''#!/usr/bin/env bash
set -euo pipefail
[[ "$1 $2" == 'plugin add' ]]
[[ "$4 $5" == '--enable --yes' ]]
mkdir -p "$HOME/.config/omarchy/plugins"
git clone --quiet "$3" "$HOME/.config/omarchy/plugins/io.github.tcballard.gardengate"
''',
    }
    for command in ("secret-tool", "kdialog", "xdg-open"):
        scripts[command] = "#!/bin/sh\nexit 0\n"
    # A compiler or package install must never be attempted in this fully provisioned fixture.
    for command in ("cargo", "rustc", "rustup", "sudo", "pacman"):
        scripts[command] = "#!/bin/sh\necho 'UNEXPECTED build/package tool' >&2\nexit 99\n"
    for name, content in scripts.items():
        path = stubs / name
        path.write_text(content)
        path.chmod(0o755)
    env = dict(os.environ, HOME=str(home), PATH=f"{stubs}:{os.environ['PATH']}")

    def run(*args, success=True):
        if simulated_user and ((args[0] == "bash" and args[1] == str(artifact)) or
                               (args[0] == "python3" and args[1].endswith("uninstall.py"))):
            wrapper = '''import os, sys
from pathlib import Path
from unittest.mock import patch
path = sys.argv.pop(1)
text = Path(path).read_text()
if path.endswith('.run'):
    text = text.split("<<'GARDENGATE_PYTHON'\\n", 1)[1].rsplit('\\nGARDENGATE_PYTHON', 1)[0]
with patch.object(os, 'geteuid', return_value=1000):
    exec(compile(text, path, 'exec'), {'__name__': '__main__'})
'''
            args = (sys.executable, "-c", wrapper, args[1], *args[2:])
        result = subprocess.run(args, env=env, capture_output=True, text=True)
        if (result.returncode == 0) != success:
            raise AssertionError(f"{args}: exit {result.returncode}\n{result.stdout}\n{result.stderr}")
        return result

    run("bash", str(artifact), "--check")
    run("bash", str(artifact))
    binary = home / ".local/bin/gardengate"
    assert "gardengate 0.0.1" in run(str(binary), "--version").stdout
    plugin = home / ".config/omarchy/plugins/io.github.tcballard.gardengate"
    assert (plugin / "BarWidget.qml").is_file()
    assert run("git", "-C", str(plugin), "remote", "get-url", "origin").stdout.strip() == (
        "https://github.com/tcballard/omarchy-plugin-gardengate.git")
    assert run("git", "-C", str(plugin), "branch", "--show-current").stdout.strip() == "main"
    run("bash", str(artifact))
    original = binary.read_bytes()
    binary.write_bytes(original + b"user change")
    assert "Modified installed file" in run("bash", str(artifact), success=False).stderr
    binary.write_bytes(original)
    inbox = home / "iCloud Drive/Omarchy Inbox"
    inbox.mkdir(parents=True)
    (inbox / "keep.txt").write_text("preserve local work")
    uninstaller = home / ".local/share/gardengate/installer/uninstall.py"
    run("python3", str(uninstaller), "--uninstall")
    assert not binary.exists()
    assert (inbox / "keep.txt").read_text() == "preserve local work"
    assert plugin.is_dir()
    print("PASS: real installer fresh/repeat, bundled Git plugin, binary, modified-file refusal, removal/data retention")
    if simulated_user:
        print("Fixture used simulated non-root identity; live user/session behavior is untested.")

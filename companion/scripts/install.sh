#!/usr/bin/env bash
set -euo pipefail
umask 077
project_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
if [[ "$(uname -m)" != x86_64 ]]; then
  echo 'The included binary requires x86_64 Linux. Build from source for another architecture.' >&2
  exit 1
fi
for dependency in rclone secret-tool systemctl; do
  command -v "$dependency" >/dev/null || { echo "Missing $dependency. On Omarchy install: sudo pacman -S --needed rclone libsecret kdialog" >&2; exit 1; }
done
rclone_version="$(rclone version | head -n 1 | sed 's/^rclone v//')"
if [[ "$(printf '%s\n' 1.75.1 "$rclone_version" | sort -V | head -n1)" != 1.75.1 ]]; then
  echo "rclone 1.75.1 or later is required; found $rclone_version. Update your package before continuing." >&2
  exit 1
fi
if [[ "$rclone_version" != 1.75.1 ]]; then
  echo "Note: transport tests were run with rclone 1.75.1; installed version is $rclone_version."
fi
command -v kdialog >/dev/null || echo 'kdialog missing: CLI works; management window needs kdialog.'
binary_path="$project_dir/bin/gardengate"
if [[ ! -f "$binary_path" ]]; then
  cargo build --manifest-path "$project_dir/Cargo.toml" --locked --release
  binary_path="$project_dir/target/release/gardengate"
fi
install -Dm755 "$binary_path" "$HOME/.local/bin/gardengate"
install -Dm644 "$project_dir/packaging/gardengate.service" "$HOME/.config/systemd/user/gardengate.service"
install -Dm644 "$project_dir/packaging/gardengate-pull.service" "$HOME/.config/systemd/user/gardengate-pull.service"
# Absolute Exec path avoids depending on the graphical session's PATH.
mkdir -p "$HOME/.local/share/applications"
if [[ "$HOME" == *'"'* || "$HOME" == *'%'* || "$HOME" == *'\\'* || "$HOME" == *'&'* || "$HOME" == *'|'* ]]; then
  echo 'Desktop launcher skipped: home path needs desktop-file escaping.'
else
  sed "s|^Exec=.*|Exec=\"$HOME/.local/bin/gardengate\" manage|" "$project_dir/packaging/io.github.tcballard.gardengate.desktop" > "$HOME/.local/share/applications/io.github.tcballard.gardengate.desktop"
fi
systemctl --user daemon-reload
printf '\nInstalled download-only preview. It has NOT signed in or started syncing.\nRun: ~/.local/bin/gardengate connect\nThen follow README.md to select a folder and approve the first download.\n'

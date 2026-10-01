#!/usr/bin/env bash
set -euo pipefail
systemctl --user disable --now gardengate.service || true
systemctl --user stop gardengate-pull.service || true
rm -f -- "$HOME/.local/bin/gardengate" "$HOME/.config/systemd/user/gardengate.service" "$HOME/.config/systemd/user/gardengate-pull.service" "$HOME/.local/share/applications/io.github.tcballard.gardengate.desktop"
systemctl --user daemon-reload
printf 'Removed application and services. Local inbox, recovery copies and encrypted account configuration retained.\n'

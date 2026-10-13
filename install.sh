#!/bin/bash
# Build and install tuf-rgb for the current user.
# Root is only needed once, for the udev rule and the "tufrgb" group.
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
install -Dm755 target/release/tuf-rgb target/release/tuf-rgbd target/release/tuf-rgb-gui -t "$HOME/.local/bin"
install -Dm644 packaging/tuf-rgbd.service "$HOME/.config/systemd/user/tuf-rgbd.service"
install -Dm644 packaging/tuf-rgb.desktop "$HOME/.local/share/applications/tuf-rgb.desktop"

echo ">> sudo: udev rule + tufrgb group"
sudo groupadd -f -r tufrgb
sudo usermod -aG tufrgb "$USER"
sudo install -Dm644 packaging/70-tuf-rgb.rules /etc/udev/rules.d/70-tuf-rgb.rules
sudo udevadm control --reload
sudo udevadm trigger --subsystem-match=leds --action=change

systemctl --user daemon-reload
systemctl --user enable tuf-rgbd.service
if id -nG | grep -qw tufrgb; then
  systemctl --user restart tuf-rgbd.service
  echo "tuf-rgbd running."
else
  echo "Log out and back in (new group membership), then: systemctl --user start tuf-rgbd"
fi

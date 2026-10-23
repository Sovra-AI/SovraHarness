#!/usr/bin/env bash
# System libraries GPUI needs on Debian/Ubuntu (from gpui-kit's script/install-linux.sh).
set -euo pipefail
sudo apt-get update
sudo apt-get install -y \
  gcc g++ clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev \
  libzstd-dev libasound2-dev libvulkan1 mesa-vulkan-drivers

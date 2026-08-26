#!/usr/bin/env bash
# Grab a PNG of the physical desktop for remote debugging.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=desktop-env.sh
source "${root}/scripts/desktop-env.sh"

out="${1:-/tmp/localtex-desktop.png}"
display_num="${DISPLAY#:}"
size="${LOCALTEX_GRAB_SIZE:-}"
if [[ -z "${size}" ]]; then
  size="$(xdpyinfo 2>/dev/null | awk '/dimensions:/ {print $2; exit}')"
fi
size="${size:-1920x1080}"
ffmpeg -y -hide_banner -loglevel error \
  -f x11grab -video_size "${size}" -i ":${display_num}.0" \
  -frames:v 1 "${out}"
echo "wrote ${out} (${size})"

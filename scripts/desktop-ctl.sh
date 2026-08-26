#!/usr/bin/env bash
# Drive the physical X11 desktop from SSH/agent: windows, mouse, keys, shot.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=desktop-env.sh
source "${root}/scripts/desktop-env.sh"

usage() {
  cat <<'EOF'
Usage: desktop-ctl.sh <command> [args]

  windows                 list windows (wmctrl)
  focus <substr>          activate first window whose title/class matches
  move <x> <y>            move pointer
  click [btn] [x y]       click (1=left, 2=middle, 3=right); optional coords
  dblclick [x y]          double-click left
  key <keysym...>         send keys, e.g. Return  or  ctrl+c
  type <text>             type a string
  getmouse                print pointer x y
  shot [path]             screenshot PNG (default /tmp/localtex-desktop.png)

Requires xdotool/wmctrl from ~/.local/opt/x11-tools or the system PATH.
EOF
}

need() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "desktop-ctl: missing $1 (install xdotool/wmctrl/xclip, or extract to ~/.local/opt/x11-tools)" >&2
    exit 1
  fi
}

cmd="${1:-}"
shift || true

case "${cmd}" in
  ""|-h|--help) usage ;;
  windows)
    need wmctrl
    wmctrl -lGxp
    ;;
  focus)
    need wmctrl
    needle="${1:-}"
    [[ -n "${needle}" ]] || { echo "desktop-ctl: focus needs a title/class substring" >&2; exit 1; }
    wmctrl -xa "${needle}" || wmctrl -a "${needle}"
    ;;
  move)
    need xdotool
    [[ $# -ge 2 ]] || { echo "desktop-ctl: move <x> <y>" >&2; exit 1; }
    xdotool mousemove --sync "$1" "$2"
    ;;
  click)
    need xdotool
    btn=1
    if [[ $# -ge 1 && "$1" =~ ^[0-9]+$ ]]; then
      btn="$1"
      shift
    fi
    if [[ $# -ge 2 ]]; then
      xdotool mousemove --sync "$1" "$2"
    fi
    xdotool click "${btn}"
    ;;
  dblclick)
    need xdotool
    if [[ $# -ge 2 ]]; then
      xdotool mousemove --sync "$1" "$2"
    fi
    xdotool click --repeat 2 --delay 80 1
    ;;
  key)
    need xdotool
    [[ $# -ge 1 ]] || { echo "desktop-ctl: key <keysym...>" >&2; exit 1; }
    xdotool key "$@"
    ;;
  type)
    need xdotool
    xdotool type --delay 12 "$*"
    ;;
  getmouse)
    need xdotool
    xdotool getmouselocation --shell
    ;;
  shot)
    exec "${root}/scripts/screenshot-desktop.sh" "${1:-/tmp/localtex-desktop.png}"
    ;;
  *)
    echo "desktop-ctl: unknown command: ${cmd}" >&2
    usage >&2
    exit 1
    ;;
esac

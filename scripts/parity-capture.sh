#!/usr/bin/env bash
# Capture trunk (or HEAD) receipts for the GPUI pin program.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=desktop-env.sh
source "${root}/scripts/desktop-env.sh"

out="${PARITY_OUT:-${root}/docs/superpowers/receipts/parity-base}"
mkdir -p "${out}"
ctl="${root}/scripts/desktop-ctl.sh"

log() { printf '%s\n' "$*"; }

need_file() {
  local path="$1"
  local min="${2:-10240}"
  [[ -f "${path}" ]] || {
    echo "parity-capture: missing ${path}" >&2
    exit 1
  }
  local bytes
  bytes="$(stat -c %s "${path}")"
  [[ "${bytes}" -gt "${min}" ]] || {
    echo "parity-capture: ${path} is ${bytes} bytes, want > ${min}" >&2
    exit 1
  }
}

cd "${root}"

if [[ "${PARITY_SKIP_BUILD:-}" != "1" ]]; then
  cargo test
  cargo build --profile dev-opt
fi

bin="${root}/target/dev-opt/localtex"
[[ -x "${bin}" ]] || {
  echo "parity-capture: missing ${bin}" >&2
  exit 1
}
stat -c %s "${bin}" | tee "${out}/size.txt"
[[ "$(cat "${out}/size.txt")" -gt 0 ]]

"${root}/scripts/run-on-desktop.sh" --stop >/dev/null 2>&1 || true
LOCALTEX_BIN="${bin}" "${root}/scripts/run-on-desktop.sh"

app_line() {
  "${ctl}" windows 2>/dev/null | awk 'tolower($8) ~ /com\.localtex\.app/ {print; exit}'
}

win_id=""
for _ in $(seq 1 40); do
  line="$(app_line || true)"
  if [[ -n "${line}" ]]; then
    win_id="$(awk '{print $1}' <<<"${line}")"
    break
  fi
  sleep 0.25
done
[[ -n "${win_id}" ]] || {
  echo "parity-capture: LocalTeX window did not appear" >&2
  exit 1
}
log "window ${win_id}"

# Drop other mapped windows that steal clicks and confuse cropped shots.
if command -v xdotool >/dev/null 2>&1; then
  while read -r oid; do
    [[ -n "${oid}" ]] || continue
    xdotool windowminimize "${oid}" || true
  done < <(xdotool search --onlyvisible --name Lody || true)
fi

raise_app() {
  "${ctl}" focus LocalTeX || true
  if [[ -n "${win_id}" ]]; then
    wmctrl -i -a "${win_id}" || true
    # Client-relative clicks need a known origin; park the window away from the dock.
    wmctrl -i -r "${win_id}" -e 0,220,80,800,560 || true
    wmctrl -i -a "${win_id}" || true
  fi
  sleep 0.2
}

raise_app

geom() {
  "${ctl}" windows | awk 'tolower($8) ~ /com\.localtex\.app/ {print $4,$5,$6,$7; exit}'
}

# Clicks are client-relative. Settings pills sit under the 44px topbar.
click_win() {
  local wx="$1"
  local wy="$2"
  raise_app
  xdotool windowactivate --sync "${win_id}"
  xdotool mousemove --window "${win_id}" --sync "${wx}" "${wy}"
  xdotool click 1
}

click_frac() {
  local fx="$1"
  local fy="$2"
  raise_app
  local w=800
  local h=560
  click_win $((w * fx / 100)) $((h * fy / 100))
}

key_app() {
  raise_app
  xdotool windowactivate --sync "${win_id}"
  sleep 0.15
  "${ctl}" key "$@"
}

shot() {
  local slug="$1"
  raise_app
  sleep 0.25
  local full="/tmp/parity-full-${slug}.png"
  "${ctl}" shot "${full}"
  local x y w h
  read -r x y w h < <(geom)
  ffmpeg -y -hide_banner -loglevel error -i "${full}" \
    -filter:v "crop=${w}:${h}:${x}:${y}" "${out}/${slug}.png"
  need_file "${out}/${slug}.png"
  log "wrote ${out}/${slug}.png crop=${w}x${h}+${x}+${y}"
}

# Lane 1. Library.
shot library

# Lanes 2-4. Settings tabs. Four pills: General, Formatting, Shortcuts, System.
key_app ctrl+comma
sleep 0.4
shot settings-general
click_win 453 68
sleep 0.3
shot settings-shortcuts
click_win 542 68
sleep 0.3
shot settings-system
key_app Escape
sleep 0.3

# Lane 5. Detail (first row is usually selected).
key_app Down
sleep 0.25
shot detail

# Lane 6. Source editor.
key_app ctrl+e
sleep 0.4
shot source-editor
key_app ctrl+e
sleep 0.3

# Lane 7. Original overlay. The strip click-to-zoom sits in the upper image pane.
click_frac 70 28
sleep 0.5
shot orig-overlay
key_app Escape
sleep 0.3

# Lane 8. Draw board.
key_app ctrl+d
sleep 0.4
shot draw
key_app Escape
sleep 0.3

# Lane 9. Hide for snip, then cancel.
key_app ctrl+shift+s
sleep 0.8
key_app Escape
sleep 0.8
raise_app
sleep 0.4
shot snip-restore

# Lane 10. Footer copy chip on a selected snip.
key_app Down
sleep 0.2
shot footer-copy

log "parity-capture: ok size=$(cat "${out}/size.txt")"

#!/usr/bin/env bash
# Launch LocalTeX on the physical desktop from SSH or a Cursor agent.
# Uses the user systemd instance so the window survives the SSH shell.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=desktop-env.sh
source "${root}/scripts/desktop-env.sh"

shell_pid="$(pgrep -u "${USER}" -x gnome-shell | head -n1 || true)"
if [[ -n "${shell_pid}" && -r "/proc/${shell_pid}/environ" ]]; then
  while IFS= read -r line; do
    case "${line}" in
      GDMSESSION=*|XDG_SESSION_DESKTOP=*|XDG_CURRENT_DESKTOP=*|XDG_SEAT=*|XDG_VTNR=*)
        export "${line?}"
        ;;
    esac
  done < <(tr '\0' '\n' < "/proc/${shell_pid}/environ")
fi

bin="${LOCALTEX_BIN:-}"
if [[ -z "${bin}" ]]; then
  profile="${LOCALTEX_PROFILE:-dev-opt}"
  bin="${root}/target/${profile}/localtex"
  if [[ ! -x "${bin}" && -x "${root}/target/release/localtex" ]]; then
    bin="${root}/target/release/localtex"
  fi
fi
if [[ ! -x "${bin}" ]]; then
  echo "run-on-desktop: missing ${bin}; run: cargo build --profile ${LOCALTEX_PROFILE:-dev-opt}" >&2
  exit 1
fi

unit="${LOCALTEX_UNIT:-localtex.service}"
if [[ "${1:-}" == "--stop" ]]; then
  systemctl --user stop "${unit}" 2>/dev/null || true
  echo "stopped ${unit}"
  exit 0
fi

systemctl --user reset-failed "${unit}" 2>/dev/null || true
systemctl --user stop "${unit}" 2>/dev/null || true

echo "launching ${bin} on DISPLAY=${DISPLAY} via ${unit}"
systemd-run --user --unit="${unit%.service}" --collect \
  --setenv=DISPLAY="${DISPLAY}" \
  --setenv=XAUTHORITY="${XAUTHORITY:-}" \
  --setenv=XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR}" \
  --setenv=DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS}" \
  --setenv=XDG_SESSION_TYPE="${XDG_SESSION_TYPE:-x11}" \
  --setenv=XDG_CURRENT_DESKTOP="${XDG_CURRENT_DESKTOP:-}" \
  --setenv=XDG_SESSION_DESKTOP="${XDG_SESSION_DESKTOP:-}" \
  --setenv=GDMSESSION="${GDMSESSION:-}" \
  --setenv=HOME="${HOME}" \
  --setenv=LOCALTEX_MODELS="${LOCALTEX_MODELS:-${HOME}/.local/share/localtex/models}" \
  "${bin}"
sleep 0.4
systemctl --user --no-pager --lines=0 status "${unit}" || true

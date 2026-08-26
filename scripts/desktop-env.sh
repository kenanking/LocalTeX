#!/usr/bin/env bash
# Discover the local graphical session so SSH/agent shells can talk to it.
set -euo pipefail

uid="$(id -u)"
export XDG_RUNTIME_DIR="${XDG_RUNTIME_DIR:-/run/user/${uid}}"
export DBUS_SESSION_BUS_ADDRESS="${DBUS_SESSION_BUS_ADDRESS:-unix:path=${XDG_RUNTIME_DIR}/bus}"

if [[ -z "${DISPLAY:-}" || -z "${XAUTHORITY:-}" ]]; then
  shell_pid="$(pgrep -u "${USER}" -x gnome-shell | head -n1 || true)"
  if [[ -n "${shell_pid}" && -r "/proc/${shell_pid}/environ" ]]; then
    while IFS= read -r line; do
      case "${line}" in
        DISPLAY=*) export DISPLAY="${line#DISPLAY=}" ;;
        XAUTHORITY=*) export XAUTHORITY="${line#XAUTHORITY=}" ;;
        WAYLAND_DISPLAY=*) export WAYLAND_DISPLAY="${line#WAYLAND_DISPLAY=}" ;;
        XDG_SESSION_TYPE=*) export XDG_SESSION_TYPE="${line#XDG_SESSION_TYPE=}" ;;
      esac
    done < <(tr '\0' '\n' < "/proc/${shell_pid}/environ")
  fi
fi

if [[ -z "${DISPLAY:-}" ]]; then
  if [[ -S /tmp/.X11-unix/X1 ]]; then
    export DISPLAY=:1
  elif [[ -S /tmp/.X11-unix/X0 ]]; then
    export DISPLAY=:0
  fi
fi

if [[ -z "${XAUTHORITY:-}" && -f "${XDG_RUNTIME_DIR}/gdm/Xauthority" ]]; then
  export XAUTHORITY="${XDG_RUNTIME_DIR}/gdm/Xauthority"
fi

if [[ -z "${DISPLAY:-}" ]]; then
  echo "desktop-env: no local X11/Wayland display found" >&2
  exit 1
fi

# User-local X11 helpers (xdotool, wmctrl, xclip) extracted without sudo.
x11_tools="${HOME}/.local/opt/x11-tools"
if [[ -d "${x11_tools}/usr/bin" ]]; then
  export PATH="${x11_tools}/usr/bin:${PATH}"
  libdir="${x11_tools}/usr/lib/x86_64-linux-gnu"
  if [[ -d "${libdir}" ]]; then
    export LD_LIBRARY_PATH="${libdir}${LD_LIBRARY_PATH:+:${LD_LIBRARY_PATH}}"
  fi
fi

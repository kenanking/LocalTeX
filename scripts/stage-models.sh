#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DEST="${1:?usage: stage-models.sh DEST}"

is_ship_root() {
  local dir="$1"
  [[ -f "$dir/opendoc/layout.onnx" ]] || [[ -f "$dir/handwriting/encoder.onnx" ]]
}

copy_ship() {
  local src="$1"
  mkdir -p "$DEST"
  rm -rf "$DEST/opendoc" "$DEST/handwriting"
  cp -a "$src/opendoc" "$DEST/opendoc"
  cp -a "$src/handwriting" "$DEST/handwriting"
  if [[ -f "$src/manifest.json" ]]; then
    cp -f "$src/manifest.json" "$DEST/manifest.json"
  fi
}

if is_ship_root "$DEST"; then
  echo "localtex: models already in $DEST"
  exit 0
fi

CANDIDATES=()
if [[ -n "${LOCALTEX_MODELS:-}" ]]; then
  CANDIDATES+=("$LOCALTEX_MODELS")
fi
case "$(uname -s)" in
  MINGW* | MSYS* | CYGWIN*)
    CANDIDATES+=("${LOCALAPPDATA:-$HOME/AppData/Local}/localtex/models")
    ;;
  *)
    CANDIDATES+=("${XDG_DATA_HOME:-$HOME/.local/share}/localtex/models")
    ;;
esac

for src in "${CANDIDATES[@]}"; do
  if is_ship_root "$src"; then
    echo "localtex: copying models from $src → $DEST"
    copy_ship "$src"
    exit 0
  fi
done

echo "localtex: no local ship packs; downloading into $DEST"
LOCALTEX_MODELS="$DEST" "$ROOT/scripts/download-models.sh"

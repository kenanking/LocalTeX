#!/usr/bin/env bash
set -euo pipefail

REPO="${LOCALTEX_MODELS_REPO:-kenanking/LocalTeX}"
TAG="${LOCALTEX_MODELS_TAG:-v0.0.0}"
OPENDOC_DIR="opendoc_dynamic_int8_20260902_45af38b"
HANDWRITING_DIR="handwriting_e10_20260831_cf27b99"
OPENDOC_TAR="${OPENDOC_DIR}.tar.gz"
HANDWRITING_TAR="${HANDWRITING_DIR}.tar.gz"
SUMS="SHA256SUMS"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "localtex: missing '$1'" >&2
    exit 1
  }
}

default_models_dir() {
  case "$(uname -s)" in
    MINGW* | MSYS* | CYGWIN*)
      echo "${LOCALAPPDATA:-$HOME/AppData/Local}/localtex/models"
      ;;
    *)
      echo "${XDG_DATA_HOME:-$HOME/.local/share}/localtex/models"
      ;;
  esac
}

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

if [[ -n "${1:-}" ]]; then
  DEST="$1"
elif [[ -n "${LOCALTEX_MODELS:-}" ]]; then
  DEST="$LOCALTEX_MODELS"
else
  DEST="$(default_models_dir)"
fi

if is_ship_root "$DEST"; then
  echo "localtex: models already in $DEST"
  exit 0
fi

CANDIDATES=()
if [[ -n "${LOCALTEX_MODELS:-}" && "${LOCALTEX_MODELS}" != "$DEST" ]]; then
  CANDIDATES+=("$LOCALTEX_MODELS")
fi
DEFAULT="$(default_models_dir)"
if [[ "$DEFAULT" != "$DEST" ]]; then
  CANDIDATES+=("$DEFAULT")
fi

for src in "${CANDIDATES[@]}"; do
  if is_ship_root "$src"; then
    echo "localtex: copying models from $src → $DEST"
    copy_ship "$src"
    exit 0
  fi
done

need tar
need sha256sum

mkdir -p "$DEST"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

download_assets() {
  local dest="$1"
  if command -v gh >/dev/null 2>&1 && {
    [[ -n "${GH_TOKEN:-${GITHUB_TOKEN:-}}" ]] || gh auth status >/dev/null 2>&1
  }; then
    gh release download "$TAG" --repo "$REPO" --dir "$dest" \
      --pattern "$OPENDOC_TAR" --pattern "$HANDWRITING_TAR" \
      --pattern "$SUMS" --pattern "manifest.json"
    return 0
  fi
  local base="https://github.com/${REPO}/releases/download/${TAG}"
  echo "localtex: gh not authenticated; trying public HTTPS (fails if the repo is private)"
  need curl
  curl -fL --retry 3 -o "$dest/$OPENDOC_TAR" "$base/$OPENDOC_TAR"
  curl -fL --retry 3 -o "$dest/$HANDWRITING_TAR" "$base/$HANDWRITING_TAR"
  curl -fL --retry 3 -o "$dest/$SUMS" "$base/$SUMS"
  curl -fL --retry 3 -o "$dest/manifest.json" "$base/manifest.json"
}

echo "localtex: downloading $REPO@$TAG → $DEST"
download_assets "$WORKDIR"

(
  cd "$WORKDIR"
  sha256sum -c "$SUMS" --ignore-missing
)

EXTRACT="$WORKDIR/extract"
mkdir -p "$EXTRACT"
tar -xzf "$WORKDIR/$OPENDOC_TAR" -C "$EXTRACT"
tar -xzf "$WORKDIR/$HANDWRITING_TAR" -C "$EXTRACT"

test -d "$EXTRACT/$OPENDOC_DIR"
test -d "$EXTRACT/$HANDWRITING_DIR"

rm -rf "$DEST/opendoc" "$DEST/handwriting" "$DEST/current" "$DEST/ship" \
  "$DEST/$OPENDOC_DIR" "$DEST/$HANDWRITING_DIR"
rm -f "$DEST"/layout.onnx "$DEST"/encoder.onnx "$DEST"/decoder.onnx \
  "$DEST"/unirec_tokenizer_mapping.json \
  "$DEST"/inktex-encoder.onnx "$DEST"/inktex-decoder-step.onnx \
  "$DEST"/inktex-vocab.json

mv "$EXTRACT/$OPENDOC_DIR" "$DEST/opendoc"
mv "$EXTRACT/$HANDWRITING_DIR" "$DEST/handwriting"

cp -f "$WORKDIR/manifest.json" "$DEST/manifest.json"
cp -f "$WORKDIR/$SUMS" "$DEST/$SUMS"

echo "localtex: OpenDoc ready in $DEST/opendoc ($OPENDOC_DIR)"
ls -lh "$DEST/opendoc"/layout.onnx "$DEST/opendoc"/encoder.onnx \
  "$DEST/opendoc"/decoder.onnx "$DEST/opendoc"/unirec_tokenizer_mapping.json
echo "localtex: handwriting ready in $DEST/handwriting ($HANDWRITING_DIR)"
ls -lh "$DEST/handwriting"/encoder.onnx \
  "$DEST/handwriting"/decoder_step.onnx \
  "$DEST/handwriting"/vocab.json

#!/usr/bin/env bash
# Fetch OpenDoc-0.1B ship + inktex WOQ weights from the GitHub release.
# Override with LOCALTEX_MODELS, LOCALTEX_MODELS_REPO, LOCALTEX_MODELS_TAG.
set -euo pipefail

REPO="${LOCALTEX_MODELS_REPO:-kenanking/LocalTeX}"
TAG="${LOCALTEX_MODELS_TAG:-v0.0.0}"
OPENDOC_TAR="opendoc-0.1b-ship.tar.gz"
INKTEX_TAR="inktex-woq.tar.gz"
SUMS="SHA256SUMS"

if [[ -n "${LOCALTEX_MODELS:-}" ]]; then
  DEST="$LOCALTEX_MODELS"
else
  DEST="${XDG_DATA_HOME:-$HOME/.local/share}/localtex/models"
fi

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "localtex: missing '$1'" >&2
    exit 1
  }
}

need tar
need sha256sum

mkdir -p "$DEST"
WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

download_assets() {
  local dest="$1"
  if command -v gh >/dev/null 2>&1 && gh auth status >/dev/null 2>&1; then
    gh release download "$TAG" --repo "$REPO" --dir "$dest" \
      --pattern "$OPENDOC_TAR" --pattern "$INKTEX_TAR" \
      --pattern "$SUMS" --pattern "manifest.json"
    return 0
  fi
  local base="https://github.com/${REPO}/releases/download/${TAG}"
  echo "localtex: gh not authenticated; trying public HTTPS (fails if the repo is private)"
  need curl
  curl -fL --retry 3 -o "$dest/$OPENDOC_TAR" "$base/$OPENDOC_TAR"
  curl -fL --retry 3 -o "$dest/$INKTEX_TAR" "$base/$INKTEX_TAR"
  curl -fL --retry 3 -o "$dest/$SUMS" "$base/$SUMS"
  curl -fL --retry 3 -o "$dest/manifest.json" "$base/manifest.json"
}

echo "localtex: downloading $REPO@$TAG → $DEST"
download_assets "$WORKDIR"

(
  cd "$WORKDIR"
  sha256sum -c "$SUMS" --ignore-missing
)

tar -xzf "$WORKDIR/$OPENDOC_TAR" -C "$DEST"
tar -xzf "$WORKDIR/$INKTEX_TAR" -C "$DEST"
cp -f "$WORKDIR/manifest.json" "$DEST/manifest.json"
cp -f "$WORKDIR/$SUMS" "$DEST/$SUMS"
(cd "$DEST" && sha256sum -c "$SUMS" --ignore-missing)

echo "localtex: OpenDoc ship ready in $DEST"
ls -lh "$DEST"/layout.onnx "$DEST"/encoder.onnx "$DEST"/decoder.onnx \
  "$DEST"/unirec_tokenizer_mapping.json "$DEST"/manifest.json
echo "localtex: inktex WOQ ready in $DEST/handwriting"
ls -lh "$DEST/handwriting"/inktex-encoder.onnx \
  "$DEST/handwriting"/inktex-decoder-step.onnx \
  "$DEST/handwriting"/inktex-vocab.json

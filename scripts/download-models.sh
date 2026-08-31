#!/usr/bin/env bash
# Fetch OpenDoc + handwriting packs from the GitHub release and wire
# ocr-pipeline-style current/ + compat links.
# Override with LOCALTEX_MODELS, LOCALTEX_MODELS_REPO, LOCALTEX_MODELS_TAG.
set -euo pipefail

REPO="${LOCALTEX_MODELS_REPO:-kenanking/LocalTeX}"
TAG="${LOCALTEX_MODELS_TAG:-v0.0.0}"
OPENDOC_DIR="opendoc_int8_20260827_45af38b"
HANDWRITING_DIR="handwriting_e10_20260831_cf27b99"
OPENDOC_TAR="${OPENDOC_DIR}.tar.gz"
HANDWRITING_TAR="${HANDWRITING_DIR}.tar.gz"
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

link_rel() {
  local target="$1"
  local link="$2"
  mkdir -p "$(dirname "$link")"
  rm -rf "$link"
  if ln -sfn "$target" "$link" 2>/dev/null; then
    return 0
  fi
  local dest
  dest="$(cd "$(dirname "$link")" && pwd)/$target"
  cp -a "$dest" "$link"
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

tar -xzf "$WORKDIR/$OPENDOC_TAR" -C "$DEST"
tar -xzf "$WORKDIR/$HANDWRITING_TAR" -C "$DEST"

# Drop the old flat / inktex-* layout so current/ is the only tree.
rm -f "$DEST"/layout.onnx "$DEST"/encoder.onnx "$DEST"/decoder.onnx \
  "$DEST"/unirec_tokenizer_mapping.json
if [[ -d "$DEST/handwriting" && ! -L "$DEST/handwriting" ]]; then
  rm -rf "$DEST/handwriting"
fi

link_rel "../$OPENDOC_DIR" "$DEST/current/opendoc"
link_rel "../$HANDWRITING_DIR" "$DEST/current/handwriting"
link_rel "$OPENDOC_DIR" "$DEST/ship"
link_rel "$HANDWRITING_DIR" "$DEST/handwriting"

cp -f "$WORKDIR/manifest.json" "$DEST/manifest.json"
cp -f "$WORKDIR/$SUMS" "$DEST/$SUMS"

echo "localtex: OpenDoc ready in $DEST/current/opendoc → $OPENDOC_DIR"
ls -lh "$DEST/$OPENDOC_DIR"/layout.onnx "$DEST/$OPENDOC_DIR"/encoder.onnx \
  "$DEST/$OPENDOC_DIR"/decoder.onnx "$DEST/$OPENDOC_DIR"/unirec_tokenizer_mapping.json
echo "localtex: handwriting ready in $DEST/current/handwriting → $HANDWRITING_DIR"
ls -lh "$DEST/$HANDWRITING_DIR"/encoder.onnx \
  "$DEST/$HANDWRITING_DIR"/decoder_step.onnx \
  "$DEST/$HANDWRITING_DIR"/vocab.json

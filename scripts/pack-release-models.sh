#!/usr/bin/env bash
# Pack ocr-pipeline current/ packs into dated tarballs for the LocalTeX release.
# Override OPENDOC_SRC, HANDWRITING_SRC, OUT.
set -euo pipefail

OPENDOC_DIR="opendoc_int8_20260827_45af38b"
HANDWRITING_DIR="handwriting_e10_20260831_cf27b99"
OCR_MODELS="${OCR_PIPELINE_MODELS:-$HOME/personal/ocr-pipeline/models}"
OPENDOC_SRC="${OPENDOC_SRC:-$(readlink -f "$OCR_MODELS/current/opendoc")}"
HANDWRITING_SRC="${HANDWRITING_SRC:-$(readlink -f "$OCR_MODELS/current/handwriting")}"
OUT="${OUT:-/tmp/localtex-models-release}"
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "localtex: missing '$1'" >&2
    exit 1
  }
}

need tar
need sha256sum

for f in layout.onnx encoder.onnx decoder.onnx unirec_tokenizer_mapping.json; do
  test -f "$OPENDOC_SRC/$f" || {
    echo "localtex: missing $OPENDOC_SRC/$f" >&2
    exit 1
  }
done
for f in encoder.onnx decoder_step.onnx vocab.json; do
  test -f "$HANDWRITING_SRC/$f" || {
    echo "localtex: missing $HANDWRITING_SRC/$f" >&2
    exit 1
  }
done

rm -rf "$OUT"
mkdir -p "$OUT/stage"
cp -a "$OPENDOC_SRC" "$OUT/stage/$OPENDOC_DIR"
cp -a "$HANDWRITING_SRC" "$OUT/stage/$HANDWRITING_DIR"
cp -f "$ROOT/models/manifest.json" "$OUT/manifest.json"

(
  cd "$OUT/stage"
  tar -czf "$OUT/${OPENDOC_DIR}.tar.gz" "$OPENDOC_DIR"
  tar -czf "$OUT/${HANDWRITING_DIR}.tar.gz" "$HANDWRITING_DIR"
)

(
  cd "$OUT"
  sha256sum "${OPENDOC_DIR}.tar.gz" "${HANDWRITING_DIR}.tar.gz" manifest.json \
    >SHA256SUMS
)

echo "localtex: packed $OUT"
ls -lh "$OUT"
cat "$OUT/SHA256SUMS"

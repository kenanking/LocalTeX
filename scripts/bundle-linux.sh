#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

need() {
  command -v "$1" >/dev/null 2>&1 || {
    echo "localtex: missing '$1'" >&2
    exit 1
  }
}

need tar
need install
need rustc

VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
test -n "$VERSION"
TARGET="$(rustc -vV | sed -n 's/^host: //p')"
ARCH="$(uname -m)"
case "$ARCH" in
  x86_64) DEB_ARCH=amd64 ;;
  aarch64) DEB_ARCH=arm64 ;;
  *)
    echo "localtex: unsupported arch $ARCH" >&2
    exit 1
    ;;
esac

OUT="${LOCALTEX_BUNDLE_OUT:-$ROOT/target/release}"
BIN="${LOCALTEX_BUNDLE_BIN:-$ROOT/target/release/localtex}"
PACKAGE="localtex-${VERSION}-${TARGET}"
mkdir -p "$OUT"

if [[ "${LOCALTEX_SKIP_BUILD:-}" != "1" ]]; then
  cargo build --locked --release --bin localtex
fi
test -x "$BIN" || {
  echo "localtex: missing $BIN" >&2
  exit 1
}

STAGE="$(mktemp -d)"
trap 'rm -rf -- "$STAGE"' EXIT
PREFIX="$STAGE/$PACKAGE"

install_app_tree() {
  local dest="$1"
  install -Dm755 "$BIN" "$dest/bin/localtex"
  install -Dm644 "$ROOT/resources/linux/com.localtex.app.desktop" \
    "$dest/share/applications/com.localtex.app.desktop"
  install -Dm644 "$ROOT/assets/icon.svg" \
    "$dest/share/icons/hicolor/scalable/apps/com.localtex.app.svg"
}

install_app_tree "$PREFIX"
install -Dm644 "$ROOT/LICENSE" "$PREFIX/share/licenses/localtex/LICENSE"
"$ROOT/scripts/stage-models.sh" "$PREFIX/share/localtex/models"
test -f "$PREFIX/share/localtex/models/opendoc/layout.onnx"
test -f "$PREFIX/share/localtex/models/handwriting/encoder.onnx"

ARCHIVE="$OUT/${PACKAGE}.tar.gz"
tar -C "$STAGE" -czf "$ARCHIVE" "$PACKAGE"
echo "localtex: wrote $ARCHIVE"

need dpkg-deb
DEB_ROOT="$STAGE/deb"
install_app_tree "$DEB_ROOT/usr"
install -Dm644 "$ROOT/LICENSE" "$DEB_ROOT/usr/share/doc/localtex/copyright"
mkdir -p "$DEB_ROOT/usr/share/localtex"
cp -a "$PREFIX/share/localtex/models" "$DEB_ROOT/usr/share/localtex/models"

SIZE_KB="$(du -sk "$DEB_ROOT" | awk '{print $1}')"
mkdir -p "$DEB_ROOT/DEBIAN"
cat >"$DEB_ROOT/DEBIAN/control" <<EOF
Package: localtex
Version: ${VERSION}
Section: graphics
Priority: optional
Architecture: ${DEB_ARCH}
Installed-Size: ${SIZE_KB}
Maintainer: Yan Tang
Homepage: https://github.com/kenanking/LocalTeX
Depends: libc6, libgcc-s1, libstdc++6, libgbm1, libxkbcommon0, libxkbcommon-x11-0, libxcb1, libxcb-xkb1, libvulkan1, libx11-6, libfontconfig1
Description: Offline screenshot OCR to TeX and Markdown
 LocalTeX snips the screen and recognizes mixed text and formulas on-device.
 This package includes the OpenDoc and handwriting ONNX packs.
EOF

DEB="$OUT/localtex_${VERSION}_${DEB_ARCH}.deb"
dpkg-deb --root-owner-group --build "$DEB_ROOT" "$DEB"
echo "localtex: wrote $DEB"

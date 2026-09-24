#!/bin/sh
# Install the blast binary for this platform into ~/.local/bin (or $BLAST_BIN_DIR).
#
# Downloads the release built for your target, verifies it against the
# checksums published with it, and refuses to install anything that does not
# match. No curl-to-shell surprises: read this file first, it is short.
set -eu

REPO="${BLAST_REPO:-lithometric/blast-protocol}"
DEST="${BLAST_BIN_DIR:-$HOME/.local/bin}"

os=$(uname -s)
arch=$(uname -m)
case "$os-$arch" in
  Darwin-arm64)  target="aarch64-apple-darwin" ;;
  Darwin-x86_64) target="x86_64-apple-darwin" ;;
  Linux-aarch64) target="aarch64-unknown-linux-gnu" ;;
  Linux-x86_64)  target="x86_64-unknown-linux-gnu" ;;
  *) echo "blast: no prebuilt binary for $os-$arch. Build it with: cargo install --git https://github.com/$REPO blast" >&2; exit 1 ;;
esac

tag="${BLAST_VERSION:-latest}"
if [ "$tag" = "latest" ]; then
  base="https://github.com/$REPO/releases/latest/download"
else
  base="https://github.com/$REPO/releases/download/$tag"
fi

tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

echo "blast: downloading $target"
curl -sSfL "$base/blast-$target" -o "$tmp/blast"
curl -sSfL "$base/SHA256SUMS" -o "$tmp/SHA256SUMS"

want=$(grep " blast-$target\$" "$tmp/SHA256SUMS" | awk '{print $1}')
if [ -z "$want" ]; then
  echo "blast: no checksum published for $target; refusing to install" >&2
  exit 1
fi
if command -v sha256sum >/dev/null 2>&1; then
  got=$(sha256sum "$tmp/blast" | awk '{print $1}')
else
  got=$(shasum -a 256 "$tmp/blast" | awk '{print $1}')
fi
if [ "$want" != "$got" ]; then
  echo "blast: checksum mismatch; refusing to install" >&2
  exit 1
fi

mkdir -p "$DEST"
chmod +x "$tmp/blast"
mv "$tmp/blast" "$DEST/blast"
echo "blast: installed to $DEST/blast"
case ":$PATH:" in
  *":$DEST:"*) ;;
  *) echo "blast: add $DEST to your PATH to use it" ;;
esac
echo "blast: now run 'blast init' inside a repository"

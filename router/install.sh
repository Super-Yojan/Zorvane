#!/bin/sh
# Download zenohd 1.10.1 and the remote-api plugin, then run the router.
# pip install eclipse-zenoh does not include zenohd. Homebrew builds often
# omit the remote-api plugin, so this script is the no-Docker path.
set -eu

VERSION=1.10.1
ROOT=$(CDPATH= cd -- "$(dirname "$0")" && pwd)
DEST=${ZORVANE_ZENOH_HOME:-"$ROOT/.cache"}
mkdir -p "$DEST"

os=$(uname -s)
arch=$(uname -m)
libc=gnu
if [ "$os" = "Linux" ] && ldd --version 2>&1 | grep -qi musl; then
  libc=musl
fi

case "$os:$arch" in
  Linux:x86_64) triple="x86_64-unknown-linux-${libc}" ;;
  Linux:aarch64) triple="aarch64-unknown-linux-${libc}" ;;
  Darwin:arm64) triple="aarch64-apple-darwin" ;;
  Darwin:x86_64) triple="x86_64-apple-darwin" ;;
  *)
    echo "No prebuilt zenohd for ${os} ${arch}." >&2
    echo "Use Docker: cd router && docker compose up" >&2
    exit 1
    ;;
esac

fetch() {
  url=$1
  zip=$2
  if [ ! -f "$zip" ]; then
    echo "Downloading $url"
    curl -fL --retry 3 -o "$zip.partial" "$url"
    mv "$zip.partial" "$zip"
  fi
  unzip -o -q "$zip" -d "$DEST"
}

fetch \
  "https://github.com/eclipse-zenoh/zenoh/releases/download/${VERSION}/zenoh-${VERSION}-${triple}-standalone.zip" \
  "$DEST/zenoh-${triple}.zip"
fetch \
  "https://github.com/eclipse-zenoh/zenoh-ts/releases/download/${VERSION}/zenoh-ts-${VERSION}-${triple}-standalone.zip" \
  "$DEST/zenoh-ts-${triple}.zip"

if [ ! -x "$DEST/zenohd" ]; then
  echo "The Zenoh archive did not contain zenohd." >&2
  exit 1
fi

# The Docker config points at /zenoh-plugins. This host config points at DEST.
sed "s#/zenoh-plugins#${DEST}#g" "$ROOT/zenoh.json5" > "$DEST/zenoh.json5"

echo "Zenoh router listening on tcp/0.0.0.0:7447 and ws://0.0.0.0:10000"
exec "$DEST/zenohd" -c "$DEST/zenoh.json5"

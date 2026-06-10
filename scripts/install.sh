#!/usr/bin/env sh
set -eu

VERSION="${TMWD_CDP_BRIDGE_VERSION:-v0.1.1}"
REPO="${TMWD_CDP_BRIDGE_REPO:-koda-claw/tmwd-cdp-bridge}"
BIN_DIR="${BIN_DIR:-$HOME/.local/bin}"
SKILL_DIR="${SKILL_DIR:-}"

os="$(uname -s)"
arch="$(uname -m)"

case "$os:$arch" in
  Darwin:arm64|Darwin:aarch64) archive="tmwd-cdp-bridge-macos-arm64.tar.gz" ;;
  Darwin:x86_64|Darwin:amd64) archive="tmwd-cdp-bridge-macos-x64.tar.gz" ;;
  Linux:x86_64|Linux:amd64) archive="tmwd-cdp-bridge-linux-x64.tar.gz" ;;
  *)
    echo "unsupported platform: $os $arch" >&2
    echo "Build from source with: cargo build --release" >&2
    exit 1
    ;;
esac

url="https://github.com/$REPO/releases/download/$VERSION/$archive"
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

echo "Downloading $url"
curl -fL -o "$tmp/$archive" "$url"
tar -xzf "$tmp/$archive" -C "$tmp"

mkdir -p "$BIN_DIR"
install -m 0755 "$tmp/tmwd-cdp-bridge" "$BIN_DIR/tmwd-cdp-bridge"
echo "Installed binary: $BIN_DIR/tmwd-cdp-bridge"

if [ -n "$SKILL_DIR" ]; then
  mkdir -p "$SKILL_DIR"
  rm -rf "$SKILL_DIR/tmwd-cdp-bridge"
  cp -R "$tmp/skills/tmwd-cdp-bridge" "$SKILL_DIR/"
  echo "Installed skill: $SKILL_DIR/tmwd-cdp-bridge"
else
  echo "Skill not installed. Set SKILL_DIR, for example:"
  echo "  SKILL_DIR=\"\$HOME/.codex/skills\" sh scripts/install.sh"
fi

echo "Next:"
echo "  tmwd-cdp-bridge install edge"
echo "  tmwd-cdp-bridge start"

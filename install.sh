#!/bin/sh
# portli installer - downloads a prebuilt release binary into ~/.local/bin.
#
#   curl -fsSL https://raw.githubusercontent.com/sulthonuladib/portli/master/install.sh | sh
#
# Options / environment:
#   VERSION=v0.1.1      Install a specific version (default: latest)
#   INSTALL_DIR=...     Install location (default: ~/.local/bin)
#
# Flags:
#   --version <v>       Same as VERSION
#   --dir <path>        Same as INSTALL_DIR
#   -h, --help          Show this help
set -eu

REPO="sulthonuladib/portli"
BIN="portli"
INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"

while [ $# -gt 0 ]; do
    case "$1" in
        --version) VERSION="${2:?--version needs a value}"; shift 2 ;;
        --dir)     INSTALL_DIR="${2:?--dir needs a value}"; shift 2 ;;
        -h|--help) sed -n '2,16p' "$0" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *) echo "portli-install: unknown argument: $1" >&2; exit 1 ;;
    esac
done

die() { echo "portli-install: $1" >&2; exit 1; }

command -v curl >/dev/null 2>&1 || die "curl is required"
command -v tar  >/dev/null 2>&1 || die "tar is required"

# --- detect platform --------------------------------------------------------
os="$(uname -s)"
arch="$(uname -m)"
case "$arch" in
    x86_64|amd64)  arch=x86_64 ;;
    aarch64|arm64) arch=aarch64 ;;
    *) die "unsupported architecture: $arch" ;;
esac
case "$os" in
    Linux)  target="$arch-unknown-linux-gnu" ;;
    Darwin) target="$arch-apple-darwin" ;;
    *) die "unsupported OS: $os (use the release archives directly on Windows)" ;;
esac
# Prefer the fully static build on musl systems (e.g. Alpine).
if [ "$os" = "Linux" ] && { [ -f /etc/alpine-release ] || ldd --version 2>&1 | grep -qi musl; }; then
    target="$arch-unknown-linux-musl"
fi

# --- resolve the version ----------------------------------------------------
if [ -z "${VERSION:-}" ]; then
    VERSION="$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
        | sed -n 's/.*"tag_name":[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1 || true)"
fi
if [ -z "${VERSION:-}" ]; then
    # Fallback: follow the /releases/latest redirect to read the tag.
    VERSION="$(curl -fsSLI -o /dev/null -w '%{url_effective}' \
        "https://github.com/$REPO/releases/latest" 2>/dev/null | sed 's#.*/tag/##' || true)"
fi
[ -n "${VERSION:-}" ] || die "could not determine the latest version"
TAG="v${VERSION#v}"

asset="$BIN-$TAG-$target.tar.gz"
url="https://github.com/$REPO/releases/download/$TAG/$asset"

echo "portli-install: installing $BIN $TAG for $target"

# --- download ---------------------------------------------------------------
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

curl -fsSL -o "$tmp/$asset" "$url" || die "download failed: $url"

# Verify the published checksum when possible.
if curl -fsSL -o "$tmp/$asset.sha256" "$url.sha256" 2>/dev/null; then
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$tmp" && sha256sum -c "$asset.sha256" >/dev/null) || die "checksum mismatch"
    elif command -v shasum >/dev/null 2>&1; then
        (cd "$tmp" && shasum -a 256 -c "$asset.sha256" >/dev/null) || die "checksum mismatch"
    fi
fi

tar -xzf "$tmp/$asset" -C "$tmp"
[ -f "$tmp/$BIN" ] || die "archive did not contain $BIN"

# --- install ----------------------------------------------------------------
mkdir -p "$INSTALL_DIR"
mv "$tmp/$BIN" "$INSTALL_DIR/$BIN"
chmod 755 "$INSTALL_DIR/$BIN"

echo "portli-install: installed $INSTALL_DIR/$BIN"

case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *)
        printf '\n%s is not on your PATH yet. Add it, for example:\n\n' "$INSTALL_DIR"
        printf '  echo '\''export PATH="%s:$PATH"'\'' >> ~/.profile\n\n' "$INSTALL_DIR"
        ;;
esac

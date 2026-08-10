#!/bin/sh
set -eu

REPOSITORY="Fractal-Tess/scorch"
VERSION=${SCORCH_VERSION:-}
INSTALL_DIR=${SCORCH_INSTALL_DIR:-}

usage() {
  cat <<'EOF'
Install Scorch release binaries.

Usage: install.sh [--version VERSION] [--install-dir DIRECTORY]

Environment:
  SCORCH_VERSION      Release version, without or with a leading "v"
  SCORCH_INSTALL_DIR  Binary installation directory

Defaults to the latest GitHub release and $HOME/.local/bin.
EOF
}

fail() {
  printf 'scorch installer: %s\n' "$*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --version)
      [ "$#" -ge 2 ] || fail "--version requires a value"
      VERSION=$2
      shift 2
      ;;
    --install-dir)
      [ "$#" -ge 2 ] || fail "--install-dir requires a value"
      INSTALL_DIR=$2
      shift 2
      ;;
    --help|-h)
      usage
      exit 0
      ;;
    *)
      fail "unknown option: $1"
      ;;
  esac
done

if command -v curl >/dev/null 2>&1; then
  download_file() {
    curl --fail --location --silent --show-error --retry 3 --output "$2" "$1"
  }
  download_stdout() {
    curl --fail --location --silent --show-error --retry 3 "$1"
  }
elif command -v wget >/dev/null 2>&1; then
  download_file() {
    wget --quiet --tries=3 --output-document="$2" "$1"
  }
  download_stdout() {
    wget --quiet --tries=3 --output-document=- "$1"
  }
else
  fail "curl or wget is required"
fi

[ "$(uname -s)" = "Linux" ] || fail "only Linux release binaries are available"

case "$(uname -m)" in
  x86_64)
    TARGET="x86_64-unknown-linux-gnu"
    ;;
  aarch64|arm64)
    TARGET="aarch64-unknown-linux-gnu"
    ;;
  *)
    fail "unsupported architecture: $(uname -m)"
    ;;
esac

if [ -z "$VERSION" ]; then
  VERSION=$(download_stdout "https://api.github.com/repos/${REPOSITORY}/releases/latest" \
    | sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"v\{0,1\}\([^"]*\)".*/\1/p' \
    | head -n 1)
  [ -n "$VERSION" ] || fail "could not determine the latest release"
fi
VERSION=${VERSION#v}
printf '%s\n' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || fail "version must use MAJOR.MINOR.PATCH"

if [ -z "$INSTALL_DIR" ]; then
  [ -n "${HOME:-}" ] || fail "HOME is unset; pass --install-dir"
  INSTALL_DIR="$HOME/.local/bin"
fi

command -v sha256sum >/dev/null 2>&1 || fail "sha256sum is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

tmpdir=$(mktemp -d)
cleanup() {
  rm -rf "$tmpdir"
  rm -f "${staged_client:-}" "${staged_server:-}"
}
trap cleanup 0 1 2 15

archive="scorch-v${VERSION}-${TARGET}.tar.xz"
root="scorch-v${VERSION}-${TARGET}"
base_url="https://github.com/${REPOSITORY}/releases/download/v${VERSION}"

printf 'Downloading Scorch v%s for %s...\n' "$VERSION" "$TARGET"
download_file "${base_url}/${archive}" "$tmpdir/$archive"
download_file "${base_url}/${archive}.sha256" "$tmpdir/${archive}.sha256"

(
  cd "$tmpdir"
  sha256sum --check --strict "${archive}.sha256"
  tar -xJf "$archive"
)

client="$tmpdir/$root/bin/scorch"
server="$tmpdir/$root/bin/scorchd"
[ -x "$client" ] || fail "archive does not contain an executable scorch binary"
[ -x "$server" ] || fail "archive does not contain an executable scorchd binary"
[ "$("$client" --version)" = "scorch $VERSION" ] \
  || fail "client version does not match v${VERSION}"
[ "$("$server" --version)" = "scorchd $VERSION" ] \
  || fail "server version does not match v${VERSION}"

mkdir -p "$INSTALL_DIR"
staged_client="$INSTALL_DIR/.scorch.$$"
staged_server="$INSTALL_DIR/.scorchd.$$"
cp "$client" "$staged_client"
cp "$server" "$staged_server"
chmod 755 "$staged_client" "$staged_server"
mv -f "$staged_client" "$INSTALL_DIR/scorch"
mv -f "$staged_server" "$INSTALL_DIR/scorchd"

printf 'Installed Scorch v%s to %s\n' "$VERSION" "$INSTALL_DIR"
case ":${PATH:-}:" in
  *":$INSTALL_DIR:"*) ;;
  *) printf 'Add %s to PATH to run scorch and scorchd.\n' "$INSTALL_DIR" ;;
esac

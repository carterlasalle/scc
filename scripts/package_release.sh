#!/usr/bin/env sh
# trace:v1 id=ops.scc-package-release work=WORK-SCC-DISTRIBUTION title="Release packaging: one source of truth for asset naming"
# Package a release: platform binary, SBOM, installer, and the checksum file
# that lists all of them.
#
#   scripts/package_release.sh <version> <outdir> [<binary>] [<sbom-source>]
#
# Produces, in <outdir>:
#   scc-<version>-<os>-<arch>          the platform binary
#   sbom-<version>.txt                 dependency inventory
#   install.sh                         the installer (platform-independent)
#   sha256-<version>-<os>-<arch>.txt   checksums for all three
#
# scripts/install.sh resolves exactly these names, and
# scripts/install_contract_test.sh builds its fixture with this script, so the
# naming has one source of truth: a rename here fails the contract test instead
# of breaking users' installs after a release.
#
# <binary> defaults to target/release/scc; <sbom-source> defaults to
# `cargo tree --edges normal`. Release binaries are built on Ubuntu 24.04 and
# macOS (see .github/workflows/release.yml).
set -eu

VERSION="${1:?usage: package_release.sh <version> <outdir> [binary] [sbom-source]}"
OUT="${2:?usage: package_release.sh <version> <outdir> [binary] [sbom-source]}"
BIN="${3:-target/release/scc}"
SBOM_SRC="${4:-}"

TAG="${VERSION#v}"
OS="$(uname -s)"
ARCH="$(uname -m)"
NAME="scc-${TAG}-${OS}-${ARCH}"
SCRIPT_DIR=$(cd "$(dirname "$0")" && pwd)

[ -f "$BIN" ] || { printf 'package_release: no binary at %s\n' "$BIN" >&2; exit 1; }

hash_files() {
    if command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$@"
    elif command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$@"
    else
        printf 'package_release: neither shasum nor sha256sum is available\n' >&2
        exit 1
    fi
}

mkdir -p "$OUT"
cp "$BIN" "$OUT/$NAME"
if [ -n "$SBOM_SRC" ]; then
    cp "$SBOM_SRC" "$OUT/sbom-${TAG}.txt"
else
    cargo tree --edges normal --format '{p}' > "$OUT/sbom-${TAG}.txt"
fi
cp "$SCRIPT_DIR/install.sh" "$OUT/install.sh"

# Hashes are recorded relative to <outdir>, which is what the release uploads
# and what install.sh matches on.
(cd "$OUT" && hash_files "$NAME" "sbom-${TAG}.txt" install.sh > "sha256-${TAG}-${OS}-${ARCH}.txt")

printf 'packaged %s:\n' "$VERSION"
ls -1 "$OUT"

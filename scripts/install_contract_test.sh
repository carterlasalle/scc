#!/usr/bin/env sh
# trace:v1 id=ops.scc-install-contract work=WORK-SCC-DISTRIBUTION title="Install contract test: installer against release asset naming"
# Network-free contract test for scripts/install.sh.
#
# scripts/package_release.sh builds a fixture release with the same asset names
# the real release publishes; the installer is then pointed at that fixture with
# SCC_DOWNLOAD_BASE / SCC_API_BASE. This is what keeps the installer and the
# release workflow in agreement — a rename on either side fails here instead of
# breaking users after a publish. The live-release canary in
# .github/workflows/ci.yml covers the published assets separately.
#
#   scripts/install_contract_test.sh
#
# Exit 0 when every check passes; nonzero on the first failure.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
WORK=$(mktemp -d "${TMPDIR:-/tmp}/scc-contract.XXXXXX")
trap 'rm -rf "$WORK"' EXIT INT TERM

VERSION=9.9.9
TAG="$VERSION"
BASE="$WORK/releases/download/v$VERSION"
OS=$(uname -s)
ARCH=$(uname -m)
PLATFORM="$OS-$ARCH"
ASSET="scc-${TAG}-${PLATFORM}"
SUMS="sha256-${TAG}-${PLATFORM}.txt"
REAL_CURL=$(command -v curl || true)

case "$PLATFORM" in
    Linux-x86_64 | Darwin-arm64) ;;
    *)
        printf 'skip: no prebuilt asset for %s (the installer exits 3 there by design)\n' "$PLATFORM"
        exit 0
        ;;
esac

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}
ok() {
    printf 'ok  %s\n' "$*"
}

make_bin() {
    # make_bin <path> <version-string>  — a stub that answers `--version`
    printf '#!/bin/sh\necho "scc %s"\n' "$2" > "$1"
    chmod +x "$1"
}

package() {
    # package <binary> — rebuild the fixture release around <binary>
    printf 'stub dependency inventory\n' > "$WORK/sbom.txt"
    "$ROOT/scripts/package_release.sh" "$VERSION" "$BASE" "$1" "$WORK/sbom.txt" > /dev/null
}

install_from_fixture() {
    # install_from_fixture <bindir> — always pinned to $VERSION, so no API call
    SCC_DOWNLOAD_BASE="file://$WORK/releases/download" \
        SCC_INSTALL_DIR="$1" \
        sh "$ROOT/scripts/install.sh" --version "$VERSION"
}

# --- T1: a valid fixture installs, verifies, and runs -------------------------
make_bin "$WORK/good" "$VERSION"
package "$WORK/good"
if ! install_from_fixture "$WORK/bin1" > "$WORK/log1" 2>&1; then
    cat "$WORK/log1" >&2
    fail "T1 install from a valid fixture should succeed"
fi
grep -q 'checksum:   verified (sha256)' "$WORK/log1" || fail "T1 checksum was not verified"
[ -x "$WORK/bin1/scc" ] || fail "T1 binary was not installed"
"$WORK/bin1/scc" --version | grep -q "scc $VERSION" || fail "T1 installed binary does not run"
ok "valid fixture installs, verifies the published checksum, and runs"

# --- T2: a tampered payload is rejected, and not overridable ------------------
printf 'tampered\n' >> "$BASE/$ASSET"
rc=0
SCC_SKIP_CHECKSUM=1 install_from_fixture "$WORK/bin2" > "$WORK/log2" 2>&1 || rc=$?
[ "$rc" -eq 4 ] || {
    cat "$WORK/log2" >&2
    fail "T2 expected exit 4 for a tampered payload, got $rc"
}
grep -q 'checksum mismatch' "$WORK/log2" || fail "T2 mismatch was not reported"
ok "tampered payload is rejected (exit 4) even with SCC_SKIP_CHECKSUM=1"

# --- T3: a checksum file without an entry honours the documented override -----
package "$WORK/good"
grep -v "$ASSET" "$BASE/$SUMS" > "$WORK/sums.trimmed"
cp "$WORK/sums.trimmed" "$BASE/$SUMS"
rc=0
install_from_fixture "$WORK/bin3" > "$WORK/log3" 2>&1 || rc=$?
[ "$rc" -eq 4 ] || {
    cat "$WORK/log3" >&2
    fail "T3 expected exit 4 when the asset has no checksum entry, got $rc"
}
grep -q 'no entry for' "$WORK/log3" || fail "T3 missing-entry message absent"
if ! SCC_SKIP_CHECKSUM=1 install_from_fixture "$WORK/bin3b" > "$WORK/log3b" 2>&1; then
    cat "$WORK/log3b" >&2
    fail "T3 SCC_SKIP_CHECKSUM=1 should allow installing without an entry"
fi
grep -q 'NOT verified' "$WORK/log3b" || fail "T3 skip was not reported"
ok "missing checksum entry fails closed (exit 4) and honours SCC_SKIP_CHECKSUM=1"

# --- T4: a binary that cannot run is an installation failure ------------------
printf '#!/bin/sh\nexit 1\n' > "$WORK/broken"
chmod +x "$WORK/broken"
package "$WORK/broken"
rc=0
install_from_fixture "$WORK/bin4" > "$WORK/log4" 2>&1 || rc=$?
[ "$rc" -eq 5 ] || {
    cat "$WORK/log4" >&2
    fail "T4 expected exit 5 when the installed binary cannot run, got $rc"
}
ok "installed binary that fails to run exits 5 instead of reporting success"

# --- T5: an unsupported platform fails closed ---------------------------------
mkdir -p "$WORK/fake"
printf '#!/bin/sh\ncase "$1" in -s) echo FreeBSD ;; -m) echo riscv64 ;; esac\n' > "$WORK/fake/uname"
chmod +x "$WORK/fake/uname"
rc=0
PATH="$WORK/fake:$PATH" sh "$ROOT/scripts/install.sh" --version "$VERSION" > "$WORK/log5" 2>&1 || rc=$?
[ "$rc" -eq 3 ] || {
    cat "$WORK/log5" >&2
    fail "T5 expected exit 3 on an unsupported platform, got $rc"
}
ok "unsupported platform exits 3 with build-from-source instructions"

# --- T6: the token never reaches the argument vector --------------------------
if [ -n "$REAL_CURL" ]; then
    mkdir -p "$WORK/shim" "$WORK/api/releases"
    cat > "$WORK/shim/curl" <<SHIM
#!/bin/sh
printf '%s\n' "\$@" >> "$WORK/args.log"
exec "$REAL_CURL" "\$@"
SHIM
    chmod +x "$WORK/shim/curl"
    printf '{"tag_name": "v%s"}\n' "$VERSION" > "$WORK/api/releases/latest"
    : > "$WORK/args.log"
    package "$WORK/good"
    if ! PATH="$WORK/shim:$PATH" \
        SCC_API_BASE="file://$WORK/api" \
        SCC_DOWNLOAD_BASE="file://$WORK/releases/download" \
        SCC_GITHUB_TOKEN="super-secret-token" \
        SCC_INSTALL_DIR="$WORK/bin6" \
        sh "$ROOT/scripts/install.sh" > "$WORK/log6" 2>&1; then
        cat "$WORK/log6" >&2
        fail "T6 install with a token should succeed"
    fi
    if grep -q 'super-secret-token' "$WORK/args.log"; then
        fail "T6 the token appeared in curl's argument vector"
    fi
    authenticated=$(grep -c '^--config$' "$WORK/args.log" || true)
    [ "$authenticated" -eq 1 ] ||
        fail "T6 expected exactly one authenticated call (the release lookup), saw $authenticated"
    ok "token goes through a 0600 --config file, is never an argv entry, and is not sent to downloads"
else
    printf 'skip: curl not installed, T6 (token handling) not exercised\n'
fi

printf '\ninstall contract: PASS\n'

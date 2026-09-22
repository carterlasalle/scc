#!/usr/bin/env sh
# trace:v1 id=ops.scc-install work=WORK-SCC-DISTRIBUTION title="Install SCC from a published release, checksum-verified"
# SCC (System Context Compiler) installer.
#
# Recommended: pin the release and verify the installer before running it.
#
#   V=0.2.6
#   P=Linux-x86_64            # or Darwin-arm64
#   B="https://github.com/carterlasalle/scc/releases/download/v${V}"
#   curl -fsSLO "${B}/install.sh" -O "${B}/sha256-${V}-${P}.txt"
#   shasum -a 256 -c "sha256-${V}-${P}.txt" --ignore-missing   # install.sh: OK
#   sh install.sh --version "${V}"
#
# The installer verifies the release binary against the published checksum, so
# the two-step form covers the payload as well. Piping the default branch
# straight into a shell executes code you have not inspected:
#
#   curl -fsSL https://raw.githubusercontent.com/carterlasalle/scc/main/scripts/install.sh | sh
#
# Options:
#   --version <v>       install a specific release (default: latest)
#   --dir <path>        install directory (default: $SCC_INSTALL_DIR or ~/.local/bin)
#   --bin-name <name>   name of the installed binary (default: scc)
#   --dry-run           resolve and print the plan; download nothing
#   -h, --help          this help
#
# Environment:
#   SCC_VERSION         same as --version
#   SCC_INSTALL_DIR     same as --dir
#   SCC_DOWNLOAD_BASE   release download base URL (default: this project's
#                       GitHub Releases path); test hook
#   SCC_API_BASE        release-lookup base URL (default: the GitHub API); test
#                       hook so the contract test can exercise the auth path
#                       without network access
#   SCC_GITHUB_TOKEN    optional GitHub token (also read from GITHUB_TOKEN) for
#                       the release lookup, to avoid unauthenticated API rate
#                       limits. It reaches curl/wget through a 0600 config file,
#                       never as a command-line argument, and is not sent to
#                       public release downloads.
#   SCC_SKIP_CHECKSUM=1 install when no checksum is available (missing asset, or
#                       no entry for this platform). Never applied to a checksum
#                       mismatch. Not recommended.
#
# Exit codes: 0 ok, 1 usage/environment error, 2 download error,
#             3 unsupported platform, 4 checksum failure,
#             5 installed binary failed to run on this host.

set -eu

REPO="carterlasalle/scc"
GITHUB="https://github.com/${REPO}"
API="https://api.github.com/repos/${REPO}"
API_BASE="${SCC_API_BASE:-$API}"

VERSION="${SCC_VERSION:-}"
INSTALL_DIR="${SCC_INSTALL_DIR:-$HOME/.local/bin}"
BIN_NAME="scc"
DRY_RUN=0

usage() {
    cat <<'EOF'
SCC (System Context Compiler) installer.

Recommended — pin the release, verify the installer, then run it:

  V=0.2.6
  P=Linux-x86_64            # or Darwin-arm64
  B="https://github.com/carterlasalle/scc/releases/download/v${V}"
  curl -fsSLO "${B}/install.sh" -O "${B}/sha256-${V}-${P}.txt"
  shasum -a 256 -c "sha256-${V}-${P}.txt" --ignore-missing
  sh install.sh --version "${V}"

Options:
  --version <v>       install a specific release (default: latest)
  --dir <path>        install directory (default: $SCC_INSTALL_DIR or ~/.local/bin)
  --bin-name <name>   name of the installed binary (default: scc)
  --dry-run           resolve and print the plan; download nothing
  -h, --help          this help

Environment:
  SCC_VERSION         same as --version
  SCC_INSTALL_DIR, SCC_DOWNLOAD_BASE, SCC_API_BASE
  SCC_GITHUB_TOKEN    optional token for the release lookup (never in argv)
  SCC_SKIP_CHECKSUM=1 install when no checksum entry exists (not recommended)

Published platforms: Linux-x86_64, Darwin-arm64. Other platforms build from
source (see docs/INSTALL.md).
EOF
    exit 0
}

die() {
    # die <exit-code> <message...>
    code="$1"
    shift
    printf 'scc installer: %s\n' "$*" >&2
    exit "$code"
}

while [ $# -gt 0 ]; do
    case "$1" in
        --version)
            [ $# -ge 2 ] || die 1 "--version needs a value"
            VERSION="$2"
            shift 2
            ;;
        --dir)
            [ $# -ge 2 ] || die 1 "--dir needs a value"
            INSTALL_DIR="$2"
            shift 2
            ;;
        --bin-name)
            [ $# -ge 2 ] || die 1 "--bin-name needs a value"
            BIN_NAME="$2"
            shift 2
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        -h | --help)
            usage
            ;;
        *)
            die 1 "unknown option: $1 (try --help)"
            ;;
    esac
done

TMP_DIR=$(mktemp -d "${TMPDIR:-/tmp}/scc-install.XXXXXX") || die 1 "cannot create a temporary directory"
trap 'rm -rf "$TMP_DIR"' EXIT INT TERM

# --- credentials ---------------------------------------------------------------
# The token goes into 0600 config files handed to curl/wget by path, so it never
# appears in the argument vector (which other users can read via ps/proc on a
# multi-user host). It is used for the release lookup only: public release
# downloads are unauthenticated.
TOKEN="${SCC_GITHUB_TOKEN:-${GITHUB_TOKEN:-}}"
AUTH_DIR=""
if [ -n "$TOKEN" ]; then
    AUTH_DIR="$TMP_DIR/auth"
    mkdir -p "$AUTH_DIR"
    chmod 700 "$AUTH_DIR"
    umask 077
    printf 'header = "Authorization: Bearer %s"
' "$TOKEN" > "$AUTH_DIR/curl.cfg"
    printf 'header = Authorization: Bearer %s
' "$TOKEN" > "$AUTH_DIR/wgetrc"
    chmod 600 "$AUTH_DIR/curl.cfg" "$AUTH_DIR/wgetrc"
    umask 022
fi

# --- HTTP ---------------------------------------------------------------------
# http_get <url> <output-file>   unauthenticated download (public assets)
# http_get_stdout <url>          authenticated request (release lookup)
if command -v curl >/dev/null 2>&1; then
    http_get_stdout() {
        if [ -n "$AUTH_DIR" ]; then
            curl -fsSL --retry 3 --retry-delay 1 --config "$AUTH_DIR/curl.cfg" "$1"
        else
            curl -fsSL --retry 3 --retry-delay 1 "$1"
        fi
    }
    http_get() {
        curl -fsSL --retry 3 --retry-delay 1 -o "$2" "$1"
    }
    url_redirect() {
        curl -fsSL -o /dev/null -w '%{url_effective}' "$1"
    }
elif command -v wget >/dev/null 2>&1; then
    http_get_stdout() {
        if [ -n "$AUTH_DIR" ]; then
            WGETRC="$AUTH_DIR/wgetrc" wget -q -O - "$1"
        else
            wget -q -O - "$1"
        fi
    }
    http_get() {
        wget -q -O "$2" "$1"
    }
    url_redirect() {
        wget -q -S -O /dev/null "$1" 2>&1 | awk '/^  Location: /{print $2}' | tail -1
    }
else
    die 1 "neither curl nor wget is available; install one and re-run"
fi

# --- platform -----------------------------------------------------------------
detect_platform() {
    case "$(uname -s)" in
        Linux) OS="Linux" ;;
        Darwin) OS="Darwin" ;;
        *) die 3 "unsupported operating system: $(uname -s)" ;;
    esac
    case "$(uname -m)" in
        x86_64 | amd64) ARCH="x86_64" ;;
        arm64 | aarch64) ARCH="arm64" ;;
        *) die 3 "unsupported architecture: $(uname -m)" ;;
    esac
    PLATFORM="${OS}-${ARCH}"
    case "$PLATFORM" in
        Linux-x86_64 | Darwin-arm64) ;;
        *)
            printf 'scc installer: no prebuilt binary is published for %s.\n\n' "$PLATFORM" >&2
            printf 'Published platforms: Linux-x86_64, Darwin-arm64.\n' >&2
            printf 'Build from source instead:\n\n' >&2
            printf '  git clone %s.git\n  cd scc\n  cargo build --release -p scc-cli\n  install -m 755 target/release/scc ~/.local/bin/scc\n\n' "$GITHUB" >&2
            exit 3
            ;;
    esac
}

# --- version ------------------------------------------------------------------
resolve_version() {
    [ -n "$VERSION" ] || {
        VERSION=$(http_get_stdout "${API_BASE}/releases/latest" 2>/dev/null |
            sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -1)
    }
    if [ -z "$VERSION" ]; then
        # API rate-limited or offline: fall back to the redirect target of
        # /releases/latest, which needs no API quota.
        target=$(url_redirect "${GITHUB}/releases/latest" 2>/dev/null || true)
        VERSION=$(printf '%s' "$target" | sed -n 's|.*/tag/\(.*\)$|\1|p')
    fi
    [ -n "$VERSION" ] || die 2 "could not determine the latest release; pass --version <v>"
    case "$VERSION" in
        v*) ;;
        *) VERSION="v${VERSION}" ;;
    esac
    NUM_VERSION="${VERSION#v}"
}

# --- checksum -----------------------------------------------------------------
sha256_of() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | awk '{print $1}'
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 "$1" | awk '{print $1}'
    elif command -v openssl >/dev/null 2>&1; then
        openssl dgst -sha256 "$1" | awk '{print $NF}'
    else
        return 1
    fi
}

verify_checksum() {
    # verify_checksum <binary-path> <checksum-file>
    expected=$(awk -v want="$ASSET" '$2 == want {print $1}' "$2" | head -1)
    [ -n "$expected" ] || return 1
    actual=$(sha256_of "$1") || return 2
    [ "$expected" = "$actual" ] || return 3
    return 0
}

# --- main ---------------------------------------------------------------------
detect_platform
resolve_version
ASSET="scc-${NUM_VERSION}-${PLATFORM}"
DOWNLOAD_BASE="${SCC_DOWNLOAD_BASE:-${GITHUB}/releases/download}"
ASSET_URL="${DOWNLOAD_BASE}/${VERSION}/${ASSET}"
SUMS_ASSET="sha256-${NUM_VERSION}-${PLATFORM}.txt"
SUMS_URL="${DOWNLOAD_BASE}/${VERSION}/${SUMS_ASSET}"
TARGET="${INSTALL_DIR}/${BIN_NAME}"

printf 'scc installer\n'
printf '  release:    %s\n' "$VERSION"
printf '  platform:   %s\n' "$PLATFORM"
printf '  binary:     %s\n' "$ASSET_URL"
printf '  checksum:   %s\n' "$SUMS_URL"
printf '  install to: %s\n' "$TARGET"

if [ "$DRY_RUN" -eq 1 ]; then
    printf '\ndry run: nothing downloaded, nothing written\n'
    exit 0
fi

printf '
downloading...
'
http_get "$ASSET_URL" "$TMP_DIR/$ASSET" || die 2 "download failed: $ASSET_URL"

checksum_ok=0
checksum_reason=""
if http_get "$SUMS_URL" "$TMP_DIR/$SUMS_ASSET" 2>/dev/null; then
    rc=0
    verify_checksum "$TMP_DIR/$ASSET" "$TMP_DIR/$SUMS_ASSET" || rc=$?
    case "$rc" in
        0) checksum_ok=1 ;;
        2) die 1 "no sha256sum, shasum, or openssl available to verify the download" ;;
        1) checksum_reason="no entry for ${ASSET} in ${SUMS_ASSET}" ;;
        *) checksum_reason="checksum mismatch for ${ASSET}" ;;
    esac
else
    checksum_reason="no published checksum at ${SUMS_URL}"
fi

if [ "$checksum_ok" -eq 1 ]; then
    printf 'checksum:   verified (sha256)
'
else
    case "$checksum_reason" in
        *mismatch*)
            # Never overridable: the download is not what the release published.
            die 4 "$checksum_reason — refusing to install. Re-run, and if it persists report it at ${GITHUB}/issues"
            ;;
    esac
    if [ "${SCC_SKIP_CHECKSUM:-0}" = "1" ]; then
        printf 'checksum:   NOT verified (%s; SCC_SKIP_CHECKSUM=1)
' "$checksum_reason" >&2
    else
        die 4 "$checksum_reason — refusing to install an unverified binary (set SCC_SKIP_CHECKSUM=1 to override)"
    fi
fi

mkdir -p "$INSTALL_DIR" || die 1 "cannot create $INSTALL_DIR"
install -m 755 "$TMP_DIR/$ASSET" "$TARGET" 2>/dev/null ||
    { cp "$TMP_DIR/$ASSET" "$TARGET" && chmod 755 "$TARGET"; } ||
    die 1 "cannot write $TARGET"

# The binary must run on this host. A mismatched libc (glibc < 2.39, musl/Alpine)
# or CPU installs cleanly and then fails on every invocation, so a failed smoke
# run is an installation failure, not a warning.
version_out=""
if ! version_out=$("$TARGET" --version 2>/dev/null); then
    printf 'scc installer: installed %s but it failed to run.
' "$TARGET" >&2
    printf 'Release binaries are built against glibc (Ubuntu 24.04); a musl/Alpine host or an older
' >&2
    printf 'glibc needs a source build. On macOS a quarantine flag can block it as well:
' >&2
    printf '  xattr -d com.apple.quarantine %s

' "$TARGET" >&2
    printf 'Build from source instead:

' >&2
    printf '  git clone %s.git
  cd scc
  cargo build --release -p scc-cli

' "$GITHUB" >&2
    exit 5
fi
printf 'installed:  %s (%s)
' "$TARGET" "$version_out"

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *)
        printf '\n%s is not on your PATH. Add it:\n\n' "$INSTALL_DIR"
        printf '  export PATH="%s:$PATH"\n\n' "$INSTALL_DIR"
        printf 'Then: cd /path/to/your/repo && scc init && scc index\n'
        exit 0
        ;;
esac

printf '\nNext:\n  cd /path/to/your/repo && scc init && scc index\n'
printf '  scc context startup          # fused startup capsule\n'
printf '  scc setup claude             # or codex / opencode / hermes / omp\n'

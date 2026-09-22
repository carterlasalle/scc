#!/usr/bin/env sh
# trace:v1 id=test.scc.docker-context work=WORK-SCC-DISTRIBUTION title="Docker build context covers every embedded file"
# The Dockerfile builds scc-cli, which embeds harness integrations with
# include_str!. Those paths live under plugins/, outside the crate tree, so a
# Dockerfile that copies only `crates/` fails to build — which is exactly what
# happened: the image had never built, and the publish workflow found out.
#
# This test is static (no Docker, no network): for every include_str! /
# include_bytes! path in the workspace it asserts that
#   1. the file exists,
#   2. a COPY line in the Dockerfile brings its top-level directory into the
#      builder stage,
#   3. .dockerignore does not exclude it.
#
#   scripts/docker_context_test.sh
#
# Portable: awk + shell builtins only (no sed backrefs — the sandbox sed
# shim rejects `"` in patterns; BSD sed wants BRE while GNU wants ERE).

set -eu
cd "$(dirname "$0")/.."

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

# include_str!("../../../plugins/omp/scc/index.ts") is relative to the file that
# contains it (crates/<crate>/src/...), so resolve each against its own file.
tmp=$(mktemp)
trap 'rm -f "$tmp"' EXIT INT TERM

# Emit "source-file bare-path" pairs via awk: split each grep -H line at the
# first ':' (source file), then strip to the quoted include path.
grep -r --include='*.rs' -H -o 'include_\(str\|bytes\)!("[^"]*")' crates/ |
    awk -F: '{ src=$1; sub(/^[^"]*"/, "", $0); sub(/".*$/, "", $0); if ($0 != "plugin_omp.rs") print src, $0 }' |
    sort -u > "$tmp"

[ -s "$tmp" ] || fail "found no include_str!/include_bytes! paths to check"

builder=$(grep -A100 'AS builder' Dockerfile | grep -B100 '^FROM ' | grep '^COPY ' || true)
[ -n "$builder" ] || fail "no builder stage found in Dockerfile"

checked=0
while read -r src_file rel; do

    # Resolve the include against its own source file's directory: the path
    # is relative to the file that contains it (e.g. plugin.wit from
    # crates/scc-plugin-api/src/lib.rs). Upward-walk past any ../ segments.
    dir=${src_file%/*}
    while :; do
        case "$rel" in
            # %/* sticks at the top level (no slash left), so empty dir
            # explicitly once dirname would return "." (the repo root).
            ../*) rel=${rel#../}; case "$dir" in */*) dir=${dir%/*};; *) dir=;; esac ;;
            *) break ;;
        esac
    done
    full="$dir/$rel"; full=${full#/}
    [ -f "$full" ] || fail "embedded path $rel (from $src_file) missing at $full"
    # Resolved paths start at crates/ or plugins/ — COPYed as whole units.
    top=${full%%/*}

    # the builder stage copies that directory
    case "$builder" in
        *" $top"*|*" $top/"*) ;;
        *) fail "Dockerfile builder stage does not COPY $top/ (needed by $rel from $src_file)" ;;
    esac

    # .dockerignore does not exclude it
    if [ -f .dockerignore ] && { grep -qx "$top" .dockerignore || grep -qx "$top/" .dockerignore; }; then
        fail ".dockerignore excludes $top/, which $rel needs"
    fi

    checked=$((checked + 1))
done < "$tmp"

printf 'docker context: %s embedded path(s) covered by the builder stage\n' "$checked"

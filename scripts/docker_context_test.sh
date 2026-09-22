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

grep -rhn --include='*.rs' -o 'include_\(str\|bytes\)!("[^"]*")' crates/ |
    sed -E 's/.*\("([^"]*)"\)/\1/' | sort -u > "$tmp"

[ -s "$tmp" ] || fail "found no include_str!/include_bytes! paths to check"

builder=$(sed -n '/AS builder/,/^FROM /p' Dockerfile)
[ -n "$builder" ] || fail "no builder stage found in Dockerfile"

checked=0
while IFS= read -r rel; do
    case "$rel" in
        plugin_omp.rs) continue ;; # a false positive from the grep above
    esac
    top=$(printf '%s' "$rel" | sed -E 's|^(\.\./)+||' | cut -d/ -f1)

    # 1. exists somewhere under that top-level directory
    found=$(find "$top" -type f -name "$(basename "$rel")" 2>/dev/null | head -1)
    [ -n "$found" ] || fail "embedded path $rel does not exist under $top/"

    # 2. the builder stage copies that directory
    printf '%s' "$builder" | grep -qE "^COPY .*[[:space:]]${top}([[:space:]]|/|$)" ||
        fail "Dockerfile builder stage does not COPY $top/ (needed by $rel)"

    # 3. .dockerignore does not exclude it
    if [ -f .dockerignore ] && grep -qE "^${top}/?$|^${top}/\*\*$" .dockerignore; then
        fail ".dockerignore excludes $top/, which $rel needs"
    fi

    checked=$((checked + 1))
done < "$tmp"

printf 'docker context: %s embedded path(s) covered by the builder stage\n' "$checked"

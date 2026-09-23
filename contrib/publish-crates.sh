#!/usr/bin/env bash
# trace:v1 id=ops.scc-publish-crates work=WORK-SCC-DISTRIBUTION title="Publish the SCC workspace crates to crates.io in dependency order"
# Publish the SCC crates to crates.io in dependency order, with the index wait
# the registry requires between a dependency and its dependents.
#
#   contrib/publish-crates.sh              # dry run: validate manifests + files
#   contrib/publish-crates.sh --execute    # real publish (needs `cargo login`)
#   contrib/publish-crates.sh --execute --no-wait
#
# Requires: cargo, curl, python3.
#
# Why a script instead of a bare `cargo publish -p scc-cli`: scc-cli depends on
# the five library crates by path+version, and crates.io must already serve each
# dependency before the next crate can be packaged. Publishing by hand in the
# wrong order fails halfway with "no matching package named ...".
set -euo pipefail
cd "$(dirname "$0")/.."

CRATES=(scc-api scc-plugin-api scc-core scc-store scc-indexer scc-graph scc-context scc-engine scc-plugin-host scc-ffi scc-cli)
EXECUTE=0
WAIT=1

usage() {
    sed -n '2,18p' "$0" | sed 's/^# \{0,1\}//'
    exit 0
}

for arg in "$@"; do
    case "$arg" in
        --execute) EXECUTE=1 ;;
        --no-wait) WAIT=0 ;;
        -h | --help) usage ;;
        *) printf 'unknown option: %s (try --help)\n' "$arg" >&2; exit 1 ;;
    esac
done

version() {
    python3 - <<'PY'
import re
print(re.search(r'^version = "([^"]+)"', open('Cargo.toml').read(), re.M).group(1))
PY
}

dep_version() {
    python3 - "$1" <<'PY'
import re, sys
name = sys.argv[1]
text = open('Cargo.toml').read()
m = re.search(rf'^{name} = \{{ path = "[^"]+", version = "([^"]+)" \}}', text, re.M)
print(m.group(1) if m else '')
PY
}

published() {
    code=$(curl -sS -o /dev/null -w '%{http_code}' -H 'User-Agent: scc-publish-script' \
        "https://crates.io/api/v1/crates/$1/$2")
    [ "$code" = "200" ]
}

wait_for_index() {
    crate="$1" ver="$2" i=0
    while [ "$i" -lt 60 ]; do
        if published "$crate" "$ver"; then
            printf '    %s %s is in the index\n' "$crate" "$ver"
            return 0
        fi
        i=$((i + 1))
        sleep 10
    done
    printf '    %s %s did not appear in the index within 10 minutes\n' "$crate" "$ver" >&2
    return 1
}

VER="$(version)"
printf 'workspace version: %s\n' "$VER"

# The version lives in two places that must agree: [workspace.package] and the
# internal dependency requirements in [workspace.dependencies].
rc=0
for c in "${CRATES[@]}"; do
    # scc-cli is the end binary (workspace member, not a workspace.dependency)
    if [ "$c" = "scc-cli" ]; then continue; fi
    dv="$(dep_version "$c")"
    if [ "$dv" != "$VER" ]; then
        printf 'MISMATCH: [workspace.dependencies] %s = %s but the workspace version is %s\n' "$c" "${dv:-<missing>}" "$VER" >&2
        rc=1
    fi
done
[ "$rc" -eq 0 ] || {
    printf 'bump both together before publishing (see .github/workflows/release.yml)\n' >&2
    exit 1
}

if [ "$EXECUTE" -eq 1 ]; then
    if [ -n "$(git status --porcelain)" ]; then
        printf 'working tree is dirty; commit before publishing\n' >&2
        exit 1
    fi
    if ! grep -qs 'crates\.io' "${CARGO_HOME:-$HOME/.cargo}/credentials.toml"; then
        printf 'no crates.io token in %s — run `cargo login` first\n' "${CARGO_HOME:-$HOME/.cargo}/credentials.toml" >&2
        exit 1
    fi
fi

for c in "${CRATES[@]}"; do
    if published "$c" "$VER"; then
        printf '==> %s %s is already on crates.io; skipping (crates.io never accepts a re-upload)\n' "$c" "$VER"
        continue
    fi
    if [ "$EXECUTE" -eq 1 ]; then
        printf '==> publishing %s %s\n' "$c" "$VER"
        cargo publish -p "$c"
        [ "$WAIT" -eq 1 ] && wait_for_index "$c" "$VER"
    else
        printf '==> dry run: %s %s\n' "$c" "$VER"
        # `cargo package --no-verify` validates the manifest and the file set
        # without building. --allow-dirty so a dry run works before the version
        # bump is committed. Cargo still resolves path dependencies from the
        # registry, so dependents can only be packaged once the crates they
        # depend on are really published — expected on a first publish.
        out=$(cargo package --no-verify --allow-dirty -p "$c" 2>&1) && rc=0 || rc=$?
        printf '%s\n' "$out"
        if [ "$rc" -ne 0 ]; then
            case "$out" in
                *"no matching package named"*)
                    printf '    expected before the first real publish: %s resolves its path dependencies\n    from crates.io, so it packages only after they are published (this script does that in order)\n' "$c"
                    ;;
                *) exit "$rc" ;;
            esac
        fi
    fi
done

if [ "$EXECUTE" -eq 1 ]; then
    printf '\npublished %s:\n' "$VER"
    for c in "${CRATES[@]}"; do
        printf '  https://crates.io/crates/%s/%s\n' "$c" "$VER"
    done
else
    printf '\ndry run complete. `cargo publish --dry-run` for scc-cli can only be\n'
    printf 'verified once the five library crates are really on crates.io.\n'
    printf 'Install after publishing: cargo install scc-cli\n'
fi

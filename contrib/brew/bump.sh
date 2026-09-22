#!/usr/bin/env bash
# trace:v1 id=ops.scc-brew-bump work=WORK-SCC-DISTRIBUTION title="Regenerate the Homebrew formula from a published release"
# Regenerate contrib/brew/system-context-compiler.rb for a new release.
#
#   contrib/brew/bump.sh            # latest GitHub release
#   contrib/brew/bump.sh 0.2.7      # specific version
#
# Requires: curl, python3.
#
# The formula installs the prebuilt binaries, so this fetches each published
# platform asset and records its real sha256. Copy the result into the tap
# (carterlasalle/homebrew-tap) after bumping.
set -euo pipefail

repo="carterlasalle/scc"
formula="$(cd "$(dirname "$0")" && pwd)/system-context-compiler.rb"
version="${1:-}"

if [ -z "$version" ]; then
    version="$(curl -fsSL -H 'Accept: application/vnd.github+json' \
        "https://api.github.com/repos/${repo}/releases/latest" |
        python3 -c 'import json,sys; print(json.load(sys.stdin)["tag_name"].lstrip("v"))')"
fi
version="${version#v}"
echo "==> ${repo} ${version}"

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT INT TERM

# platform-asset-name:platform-label pairs the formula knows about
assets="Darwin-arm64 Linux-x86_64"

for platform in $assets; do
    asset="scc-${version}-${platform}"
    url="https://github.com/${repo}/releases/download/v${version}/${asset}"
    echo "    fetching ${asset}"
    if ! curl -fsSL -o "${tmp}/${asset}" "$url"; then
        echo "    MISSING: ${url} (the release does not publish this platform)" >&2
        exit 1
    fi
done

python3 - "$formula" "$version" "$tmp" $assets <<'PY'
import hashlib
import pathlib
import re
import sys

formula, version, tmp = sys.argv[1], sys.argv[2], sys.argv[3]
platforms = sys.argv[4:]

text = pathlib.Path(formula).read_text(encoding="utf-8")
text = re.sub(r'^  version "[^"]+"$', f'  version "{version}"', text, count=1, flags=re.M)

for platform in platforms:
    asset = f"scc-{version}-{platform}"
    digest = hashlib.sha256(pathlib.Path(tmp, asset).read_bytes()).hexdigest()
    url = f"https://github.com/carterlasalle/scc/releases/download/v{version}/{asset}"

    # rewrite this platform's url line, then the sha256 line directly below it
    pattern = re.compile(
        rf'^(\s*)url "https://github\.com/carterlasalle/scc/releases/download/v[^"]*{re.escape(platform)}"$',
        re.M,
    )
    if not pattern.search(text):
        raise SystemExit(f"no url line for {platform} in {formula}")
    text = pattern.sub(lambda m: f'{m.group(1)}url "{url}"', text, count=1)

    pattern = re.compile(
        rf'(\n\s*url "{re.escape(url)}"\n\s*sha256 ")[0-9a-f]+(")',
        re.M,
    )
    if not pattern.search(text):
        raise SystemExit(f"no sha256 line after the {platform} url in {formula}")
    text = pattern.sub(lambda m: f"{m.group(1)}{digest}{m.group(2)}", text, count=1)

    print(f"    {platform}: {digest}")

pathlib.Path(formula).write_text(text, encoding="utf-8")
PY

echo "==> ${formula}"
grep -E "^[[:space:]]*(version|url|sha256)" "$formula"
echo
echo "next: copy it into the tap and push"
echo "  git -C <tap> rm -f Formula/system-context-compiler.rb 2>/dev/null || true"
echo "  cp '${formula}' <tap>/Formula/system-context-compiler.rb"

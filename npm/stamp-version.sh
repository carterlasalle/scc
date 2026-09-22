#!/usr/bin/env bash
# trace:v1 id=ops.scc-npm-stamp work=WORK-SCC-DISTRIBUTION title="Stamp the npm package versions from a release tag"
# Stamp the npm package versions for a release.
#
#   npm/stamp-version.sh 0.2.7
#
# Sets the version in all three CLI packages and — critically — rewrites the
# launcher's optionalDependencies to the same version. `npm version` alone only
# changes a package's own version, so a launcher published as 0.2.7 would still
# point at the 0.2.6 platform packages and npm would install the old binary.
#
# The release workflow calls this before publishing; it is idempotent and safe to
# run by hand.
set -euo pipefail
cd "$(dirname "$0")/.."

version="${1:-}"
if [ -z "$version" ]; then
    printf 'usage: npm/stamp-version.sh <version>   (e.g. 0.2.7, no leading v)\n' >&2
    exit 1
fi
version="${version#v}"

node - "$version" <<'JS'
const fs = require("node:fs");
const version = process.argv[2];

const packages = ["npm/cli", "npm/cli-linux-x64", "npm/cli-darwin-arm64"];
for (const dir of packages) {
  const file = `${dir}/package.json`;
  const pkg = JSON.parse(fs.readFileSync(file, "utf8"));
  pkg.version = version;
  if (pkg.optionalDependencies) {
    for (const dep of Object.keys(pkg.optionalDependencies)) {
      pkg.optionalDependencies[dep] = version;
    }
  }
  fs.writeFileSync(file, JSON.stringify(pkg, null, 2) + "\n");
  console.log(`${file} -> ${version}`);
}
JS

printf '\nstamped %s. Publish order: platform packages first, then npm/cli.\n' "$version"

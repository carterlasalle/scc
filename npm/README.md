<!-- trace:v1 id=doc.scc-npm type=document work=WORK-SCC-DISTRIBUTION -->
# scc on npm

```bash
npm install -g @carterlasalle/scc
scc --version
npx -y @carterlasalle/scc mcp        # the MCP server, ten semantic tools
```

<!-- trace:v1 id=doc.scc-npm.why-the-scope work=WORK-SCC-DISTRIBUTION -->
## Why the scope

`npm install -g scc` installs an unrelated 2013 SeaJS bundler — the bare name is
taken, as it is on crates.io, PyPI and Homebrew. This project publishes under the
`@carterlasalle` scope instead:

| Registry | Artifact | Install |
|---|---|---|
| npm | `@carterlasalle/scc` (CLI, this directory) | `npm install -g @carterlasalle/scc` |
| crates.io | `scc-cli` | `cargo install scc-cli` |
| Homebrew | `carterlasalle/tap/system-context-compiler` | `brew install carterlasalle/tap/system-context-compiler` |
| GitHub Releases | `scc-<version>-<platform>` + installer | `curl -fsSL …/scripts/install.sh \| sh` |
| GitHub Container Registry | `ghcr.io/carterlasalle/scc` | `docker run ghcr.io/carterlasalle/scc --version` |

<!-- trace:v1 id=doc.scc-npm.layout work=WORK-SCC-DISTRIBUTION -->
## Layout

| Path | Package | Contents |
|---|---|---|
| `npm/cli` | `@carterlasalle/scc` | the `scc` launcher (`bin/scc.js`) plus `optionalDependencies` on the platform packages |
| `npm/cli-linux-x64` | `@carterlasalle/scc-linux-x64` | `bin/scc` for Linux x64, `os`/`cpu` gated |
| `npm/cli-darwin-arm64` | `@carterlasalle/scc-darwin-arm64` | `bin/scc` for macOS arm64, `os`/`cpu` gated |

The launcher resolves the platform package, execs the binary and forwards argv,
stdio and the exit code, so `npx -y @carterlasalle/scc mcp` behaves exactly like a
native install. Set `SCC_BIN=/path/to/scc` to run a locally built binary instead.

<!-- trace:v1 id=doc.scc-npm.publishing work=WORK-SCC-DISTRIBUTION -->
## Publishing

`.github/workflows/release.yml` publishes all three packages on a `v*` tag: the
binary from the release artifacts is copied into the platform package, the
version is stamped from the tag (`npm version --no-git-tag-version`), and each
package is published with `--provenance` using the `NPM_TOKEN` secret. The
platform package must be published before the launcher, or the launcher's
`optionalDependencies` would point at a version that does not exist yet.

<!-- trace:v1 id=doc.scc-publishing type=document work=WORK-SCC-DISTRIBUTION documents=REQ-SCC-API -->
# Publishing SCC

Every channel SCC ships on, who publishes it, and what credential it needs.

The bare name `scc` is taken on npm, crates.io, PyPI and Homebrew by unrelated
projects (`npm install -g scc` is a 2013 SeaJS bundler, `brew install scc` is
[boyter/scc](https://github.com/boyter/scc), a Go line counter), so every
channel below publishes under a scoped or qualified name.

| Channel | Artifact | Published by | Credential | Status |
|---|---|---|---|---|
| GitHub Releases | `scc-<v>-<os>-<arch>`, `install.sh`, `sbom-<v>.txt`, `sha256-<v>-<os>-<arch>.txt` | `release.yml` (`dist` + `release` jobs) on a `v*` tag | workflow token | ✅ live |
| npm | `scc-sdk` (TypeScript SDK) | `release.yml` `npm` job | `NPM_TOKEN` secret | ✅ live |
| PyPI | `scc-sdk` (Python SDK) | `release.yml` `pypi` job | trusted publishing, no token | ✅ live |
| GHCR | `ghcr.io/carterlasalle/scc` | `publish-image.yml` | workflow token | ✅ live |
| Homebrew tap | `carterlasalle/tap/system-context-compiler` | manual — see below | a PAT that can push to the tap repo | ✅ live (0.2.6) |
| MCP registry | `io.github.carterlasalle/scc` | manual — see below | GitHub login via `mcp-publisher` | ⏳ blocked on npm CLI package (below) |
| npm | `@carterlasalle/scc` + `@carterlasalle/scc-linux-x64` + `@carterlasalle/scc-darwin-arm64` | `release.yml` `npm-cli` job | `NPM_TOKEN` secret | ❌ never published (404 as of 2026-09-23) |
| npm | `@carterlasalle/omp-scc` (Oh My Pi extension) | `release.yml` `npm-omp` job | `NPM_TOKEN` secret | ❌ never published (404 as of 2026-09-23) |
| crates.io | `scc-core`, `scc-store`, `scc-indexer`, `scc-graph`, `scc-context`, `scc-cli` | `release.yml` `crates` job (gated on `CRATES_PUBLISH`) | `CARGO_REGISTRY_TOKEN` secret + `CRATES_PUBLISH` variable | ❌ never published (`scc-cli` does not exist as of 2026-09-23) |

The ❌ rows are wired in the workflow but have never produced a registry
entry — do not document them as install paths until a tagged release turns
them green. Suspect for the npm rows: the `npm-cli` / `npm-omp` jobs have no
`needs: [release]` ordering and no failure gate surfaced in the release
summary, so a silent skip looks like success. For crates: the gate variable
was likely never set to `true`. Next tag: watch those three jobs explicitly,
then flip their rows above.

<!-- trace:v1 id=doc.scc-publishing.one-time-setup work=WORK-SCC-DISTRIBUTION -->
## One-time setup

The crates.io token comes from <https://crates.io/settings/tokens>; the npm
token is an automation (classic) token. `CRATES_PUBLISH` is the `if:` gate:

```bash
gh secret set CARGO_REGISTRY_TOKEN -R carterlasalle/scc
gh variable set CRATES_PUBLISH --body true -R carterlasalle/scc
gh secret set NPM_TOKEN -R carterlasalle/scc
curl -L "https://github.com/modelcontextprotocol/registry/releases/latest/download/mcp-publisher_$(uname -s | tr '[:upper:]' '[:lower:]')_$(uname -m).tar.gz" | tar xz mcp-publisher
./mcp-publisher login github            # add --token <PAT> to skip the browser flow
```

The last two lines install and authenticate the MCP registry publisher. Use the
**official release binary**, not the snap: the snap ships mcp-publisher 1.1.0
(third-party) which rejects the current server.json schema even for `--help`, and
its token storage predates the `~/.config/mcp-publisher/` location the current
tool reads.

`CRATES_PUBLISH` is a **variable**, not a secret, on purpose: `secrets` is not
allowed in `if:` expressions, and a workflow that references it there fails at
0 seconds with no job log.

<!-- trace:v1 id=doc.scc-publishing.cutting-a-release work=WORK-SCC-DISTRIBUTION -->
## Cutting a release

1. Bump every version that is not stamped by CI:

   | File | Field |
   |---|---|
   | `Cargo.toml` | `[workspace.package] version` **and** the five internal dependency versions in `[workspace.dependencies]` |
   | `sdk/python/pyproject.toml` | `version` |
   | `sdk/typescript/package.json` | `version` |
   | `plugins/omp/scc/package.json` | `version` (also embedded in the binary by `scc setup omp`) |
   | `plugins/hermes/scc/plugin.yaml` | `version` |
   | `server.json` | `version` |

   The npm CLI packages and the Oh My Pi extension are stamped from the tag by
   `npm/stamp-version.sh` inside the release workflow, so they cannot drift.

2. Validate the crates locally: `./contrib/publish-crates.sh` (dry run — checks
   the version agreement and packages every crate).
3. Tag and push: `git tag v0.2.7 && git push origin v0.2.7`. The workflow builds
   both platforms, creates the release, and publishes to crates.io / npm / PyPI.
4. Post-release, per channel:
   - **Homebrew**: `./contrib/brew/bump.sh 0.2.7`, then copy
     `contrib/brew/system-context-compiler.rb` into
     `carterlasalle/homebrew-tap` as `Formula/system-context-compiler.rb`.
   - **MCP registry**: `./mcp-publisher login github && ./mcp-publisher publish`
     (reads `server.json`).
   - **Container image**: published automatically by `publish-image.yml` on the
     tag; verify with the pull check below.

<!-- trace:v1 id=doc.scc-publishing.verify-each-channel work=WORK-SCC-DISTRIBUTION -->
## Verify each channel

A green job is not proof a package is installable. These are the checks that
read the registry back:

```bash
# live channels only — the ❌ rows above have nothing to check until a tag publishes them
npm view scc-sdk version | tail -1

curl -sS https://pypi.org/pypi/scc-sdk/json | python3 -c 'import json,sys; print(json.load(sys.stdin)["info"]["version"])'

gh release view v0.2.7 -R carterlasalle/scc --json assets --jq '.assets[].name'

echo "$GITHUB_TOKEN" | docker login ghcr.io -u carterlasalle --password-stdin
docker pull ghcr.io/carterlasalle/scc:v0.2.7 && docker run --rm ghcr.io/carterlasalle/scc:v0.2.7 --version

curl -sS "https://registry.modelcontextprotocol.io/v0/servers?search=io.github.carterlasalle" | python3 -m json.tool | head -40

brew update && brew info carterlasalle/tap/system-context-compiler
```

<!-- trace:v1 id=doc.scc-publishing.directory-listings work=WORK-SCC-DISTRIBUTION -->
## Directory listings

Where people look for MCP servers, what each needs, and what is blocking it.

| Directory | Mechanism | Status |
|---|---|---|
| [awesome-mcp-servers](https://github.com/punkpeye/awesome-mcp-servers) | PR adding one row to `README.md` (agent PRs add 🤖🤖🤖 to the title for fast-tracking) | PR opened: <https://github.com/punkpeye/awesome-mcp-servers/pull/14845> |
| [Official MCP registry](https://registry.modelcontextprotocol.io) | `mcp-publisher` + `server.json`; the registry validates that the referenced npm package exists | authenticated, and blocked only by the package: `NPM package '@carterlasalle/scc' not found (status: 404)`. After the next tagged release, `mcp-publisher publish` completes it |
| [Glama](https://glama.ai/mcp/servers) | crawls public GitHub repos that expose an MCP server; no submission form | automatic once the repo is indexed |
| [mcp.so](https://www.mcp.so/submit) | submission form; free tier needs a signed-in account (a paid $39 tier publishes immediately) | needs an account |
| [Smithery](https://smithery.ai) | `smithery mcp publish` after `smithery auth login`, or the web form | needs an account |
| [PulseMCP](https://www.pulsemcp.com/submit) | submission form behind Cloudflare, requires a browser session | needs an account |

Order matters: the MCP registry entry points at `@carterlasalle/scc`, so the npm
package has to be published (next tag) before that listing can be created. The
awesome-list PR has no such dependency and is already open.

After publishing to the MCP registry, verify the entry is served:

```bash
curl -sS "https://registry.modelcontextprotocol.io/v0/servers?search=io.github.carterlasalle" | python3 -m json.tool | head -40
```

<!-- trace:v1 id=doc.scc-publishing.installer-asset-contract work=WORK-SCC-DISTRIBUTION -->
## Installer asset contract

`scripts/install.sh` resolves exactly the names
`scripts/package_release.sh` produces. That contract is tested offline by
`scripts/install_contract_test.sh` (local fixture, no network) and against the
published release by the `install-smoke` canary in `ci.yml`, so a rename on
either side fails CI instead of breaking users after a publish.

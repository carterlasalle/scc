<!-- trace:v1 id=doc.scc-install type=document work=WORK-SCC-DISTRIBUTION documents=REQ-SCC-API -->

# Installing SCC

`docs/INSTALL.md` is the authoritative install reference: supported platforms,
every install path, verification, harness setup, and troubleshooting.

## Install (recommended)

Pin the release, verify the installer, then run it — nothing executes before
you have checked it against the release's own checksum file:

```bash
V=0.2.6
P=Linux-x86_64            # or Darwin-arm64
B="https://github.com/carterlasalle/scc/releases/download/v${V}"
curl -fsSLO "${B}/install.sh" -O "${B}/sha256-${V}-${P}.txt"
shasum -a 256 -c "sha256-${V}-${P}.txt" --ignore-missing   # install.sh: OK
sh install.sh --version "${V}"
```

That verifies the installer itself before it runs, and the installer then
verifies the release binary against the same checksum file. Pick the release you
want — the latest is listed at
<https://github.com/carterlasalle/scc/releases/latest>.

A one-liner convenience form exists, but it executes whatever is on the default
branch at that moment, so nothing can be verified before it runs:

```bash
curl -fsSL https://raw.githubusercontent.com/carterlasalle/scc/main/scripts/install.sh | sh
```

The installer:

1. detects the platform (`Linux-x86_64`, `Darwin-arm64`);
2. resolves the release tag (`--version <v>`, or the latest release via the API);
3. downloads `scc-<version>-<platform>` from GitHub Releases;
4. verifies it against the published `sha256-<version>-<platform>.txt` and
   **refuses to install on mismatch** — that refusal is not overridable;
5. installs to `~/.local/bin/scc` (override with `--dir` or `SCC_INSTALL_DIR`),
   runs `scc --version` as a smoke test and fails the install if the binary
   cannot run on this host, then prints the PATH line if needed.

It is short, has no dependencies beyond `curl`/`wget` and
`sha256sum`/`shasum`/`openssl`, and never uses `sudo`:
<https://github.com/carterlasalle/scc/blob/main/scripts/install.sh>

### Installer options

| Flag | Environment | Meaning |
|---|---|---|
| `--version <v>` | `SCC_VERSION` | Install a specific release (e.g. `--version 0.2.6`); default is the latest |
| `--dir <path>` | `SCC_INSTALL_DIR` | Install directory; default `~/.local/bin` |
| `--bin-name <name>` | — | Installed binary name; default `scc` |
| `--dry-run` | — | Print the resolved release, asset URLs, and target path; download nothing |
| — | `SCC_GITHUB_TOKEN` / `GITHUB_TOKEN` | Token for the release lookup, to avoid unauthenticated API rate limits. Passed to curl/wget through a `0600` config file, never as a command-line argument, and never sent to the public release downloads |
| — | `SCC_SKIP_CHECKSUM=1` | Install when no checksum is available (missing asset, or no entry for this platform). Never applies to a checksum *mismatch*. Not recommended |
| — | `SCC_DOWNLOAD_BASE`, `SCC_API_BASE` | Test hooks that point the installer at a local fixture; used by `scripts/install_contract_test.sh` |

Exit codes: `0` success, `1` usage/environment error, `2` download error,
`3` unsupported platform, `4` checksum failure (missing or mismatched),
`5` the installed binary failed to run on this host.

### Verify the install

```bash
scc --version        # scc 0.2.6
scc doctor           # integration registry health (offline by default)
```

## Manual install (no script)

```bash
VERSION=0.2.6
PLATFORM=Linux-x86_64          # or Darwin-arm64
BASE="https://github.com/carterlasalle/scc/releases/download/v${VERSION}"

curl -fLO "${BASE}/scc-${VERSION}-${PLATFORM}"
curl -fLO "${BASE}/sha256-${VERSION}-${PLATFORM}.txt"
sha256sum -c "sha256-${VERSION}-${PLATFORM}.txt" --ignore-missing          # Linux
shasum -a 256 -c "sha256-${VERSION}-${PLATFORM}.txt" --ignore-missing     # macOS
mkdir -p ~/.local/bin
install -m 755 "scc-${VERSION}-${PLATFORM}" ~/.local/bin/scc
```

The checksum file lists the binary, the SBOM and `install.sh`, so
`--ignore-missing` verifies only what you actually downloaded — on macOS that
flag is what keeps the command from failing on the files you skipped. Every
release also publishes `sbom-<version>.txt` (dependency inventory) and
`install.sh` (the installer, so the pinned flow above can verify it), and is
built from the tagged commit; the release notes carry the changelog.

## Supported platforms

| Platform | Prebuilt binary | Notes |
|---|---|---|
| Linux x86_64 | ✅ | glibc (built on Ubuntu; runs on glibc ≥ 2.39) |
| macOS arm64 (Apple silicon) | ✅ | signed-style checksums published; notarization is not yet set up, so Gatekeeper may require `xattr -d com.apple.quarantine ~/.local/bin/scc` |
| Linux arm64, macOS x86_64, Windows | ❌ | Build from source (below). On Windows use WSL2 and the Linux x86_64 binary |

## Build from source

```bash
git clone https://github.com/carterlasalle/scc.git
cd scc
cargo build --release -p scc-cli          # → target/release/scc
cargo test --workspace                    # full suite
cargo clippy --workspace -- -D warnings   # CI gate
install -m 755 target/release/scc ~/.local/bin/scc
```

Rust stable is the only hard requirement. Optional extras: `pyright` +
`typescript-language-server` (LSP resolution), `ollama` or any
OpenAI-compatible embedding endpoint (semantic ranking), `zstd` (CBM adapter),
`python3` + `node` (SDK and plugin tests).

## Docker

No image is published to a registry yet; build the checked-in `Dockerfile`:

```bash
docker build -t scc .
```

Two behaviors drive everything below, both verified against the daemon
(`crates/scc-cli/src/httpd.rs`):

- it binds `127.0.0.1:7777` by default and **refuses a non-loopback bind unless
  `SCC_ALLOW_REMOTE_LISTEN=1`** is set — there is no authentication, so the
  opt-in is deliberate. A plain `-p 7777:7777` therefore maps a port nothing is
  listening on;
- state has to live outside the read-only repository mount
  (`SCC_STATE_DIR=/data`, set in the image), otherwise indexing fails with
  `Read-only file system`.

Index once, then serve:

```bash
docker run --rm -w /repo -v "$PWD:/repo:ro" -v scc-data:/data scc index
```

**Linux — share the host network.** The default loopback bind is then reachable
from the host, and nothing is exposed beyond this machine:

```bash
docker run --rm --network host -v "$PWD:/repo:ro" -v scc-data:/data scc serve
```

`http://127.0.0.1:7777/healthz` answers from the host.

**Docker Desktop (macOS/Windows) — publish a port.** `--network host` is not
available there, so the daemon must be told to bind `0.0.0.0` inside the
container. That needs a config saying so, plus the opt-in. The config lives at
`<repo>/.scc/config.yaml`, which means the directory has to exist in the
repository before the container starts — Docker cannot create that mountpoint
under a read-only mount (`create mountpoint ... read-only file system`):

```bash
cd /path/to/your/repo
scc init                                  # creates .scc/ (the mountpoint)
mkdir -p /tmp/scc-container/.scc
printf 'security:\n  listen: 0.0.0.0:7777\n' > /tmp/scc-container/.scc/config.yaml

docker run --rm -w /repo \
  -v "$PWD:/repo:ro" \
  -v /tmp/scc-container/.scc:/repo/.scc:ro \
  -v scc-data:/data \
  -e SCC_ALLOW_REMOTE_LISTEN=1 \
  -p 127.0.0.1:7777:7777 \
  scc serve
```

The container-only config is mounted over `/repo/.scc` so your repository's own
config stays untouched (the daemon reads `security.listen` from that path).
Keep the host side on `127.0.0.1`: the daemon has no authentication and logs
`warning: unauthenticated SCC daemon on non-loopback 0.0.0.0:7777` when it
accepts a non-loopback bind. See
[DEPLOYMENT_AND_INFRA.md](DEPLOYMENT_AND_INFRA.md).

## Package names — read this before `npm install -g scc`

The name `scc` is taken on every public package registry by unrelated projects.
Installing "scc" from those registries gets you a different tool:

| Command | What it actually installs |
|---|---|
| `npm install -g scc` | A 2013 SeaJS combo/compress tool (`gxcsoccer/scc`), last published 2022 |
| `brew install scc` | `boyter/scc` — a Go line counter (Sloc, Cloc and Code) |
| `cargo install scc` | `scalable-concurrent-containers` on crates.io |
| `pip install scc` | Open Microscopy OME workflow tools |

This project ships as:

| Artifact | Registry | Install |
|---|---|---|
| `scc` CLI | GitHub Releases (via `scripts/install.sh`) | see above |
| TypeScript SDK `scc-sdk` | [npm](https://www.npmjs.com/package/scc-sdk) | `npm install scc-sdk` |
| Python SDK `scc-sdk` | [PyPI](https://pypi.org/project/scc-sdk/) | `pip install scc-sdk` |

A crates.io / npm / Homebrew formula for the CLI itself is not published yet;
the installer is the supported path.

## Harness integrations

Install SCC in the repository first (`scc init && scc index`), then wire your
harness — each command is idempotent and only touches that harness's config:

| Harness | Command | What it installs |
|---|---|---|
| Claude Code | `scc setup claude` | SessionStart capsule, task-pack injection, post-edit refresh, PreCompact checkpoint |
| Codex | `scc setup codex` | `AGENTS.md` with the capsule and authority ordering |
| OpenCode | `scc setup opencode` | `AGENTS.md` + `.opencode/opencode.json` wiring the SCC MCP server |
| Hermes | `scc setup hermes` | Native plugin (10 tools) + bundled skill |
| Oh My Pi (OMP) | `scc setup omp` | Native extension, MCP, skill, `AGENTS.md` |
| Pi | `scc setup pi` | Project-local `.pi/extensions/scc` |
| All detected | `scc setup` | Auto-detects installed harnesses; `scc setup all` skips detection |

MCP-only clients: run `scc mcp` on stdio and register the ten semantic tools
(`system_atlas`, `system_overview`, `task_context`, `component_context`,
`flow_context`, `impact_context`, `verify_context`, `system_context`,
`surface_map`, `structural_source`) — see
[API_AND_INTEGRATIONS.md](API_AND_INTEGRATIONS.md).

## First run

```bash
cd /path/to/your/repo
scc init                       # .scc/config.yaml + database
scc index                      # cold index; incremental afterwards
scc overview                   # startup capsule
scc context task "add retry to the payment webhook"
```

## CI usage

```yaml
- uses: actions/checkout@v4
- name: Install SCC (pinned + verified)
  run: |
    V=0.2.6
    P=Linux-x86_64
    B="https://github.com/carterlasalle/scc/releases/download/v${V}"
    curl -fsSLO "${B}/install.sh" -O "${B}/sha256-${V}-${P}.txt"
    shasum -a 256 -c "sha256-${V}-${P}.txt" --ignore-missing
    sh install.sh --version "${V}"
    echo "$HOME/.local/bin" >> "$GITHUB_PATH"
- run: scc init && scc index
- run: scc ci check            # invariants + drift severity policy; nonzero on violation
```

If you build SCC from source in your own CI, run `./scripts/install.sh` from
that checkout instead of downloading anything.

## Uninstall

```bash
rm -f ~/.local/bin/scc         # the binary
rm -rf .scc                    # per-repository index and config
```

Harness integrations are ordinary files: `scc setup <harness>` prints what it
wrote, so you can remove the SCC block from `AGENTS.md`, `.claude/settings.json`,
`.opencode/opencode.json`, or the Hermes/OMP plugin directory.

## Troubleshooting

| Symptom | Cause and fix |
|---|---|
| `scc: command not found` | `~/.local/bin` is not on `PATH`: `export PATH="$HOME/.local/bin:$PATH"` |
| `unsupported platform` (exit 3) | No prebuilt binary for that OS/arch — build from source |
| `no published checksum ... refusing to install` (exit 4) | No checksum asset, or no entry for your platform in it. Prefer a release that publishes checksums, or build from source; `SCC_SKIP_CHECKSUM=1` overrides this case only |
| `checksum mismatch` (exit 4) | Corrupted or tampered download — re-run; if it persists, report it at <https://github.com/carterlasalle/scc/issues>. This one is never overridable |
| `installed ... but it failed to run` (exit 5) | The release binary does not match this host: glibc < 2.39, musl/Alpine, or an unsupported CPU. Build from source. On macOS clear the quarantine flag: `xattr -d com.apple.quarantine ~/.local/bin/scc` |
| macOS: "cannot be opened because the developer cannot be verified" | `xattr -d com.apple.quarantine ~/.local/bin/scc` (notarization not yet set up) |
| Installer hangs or 403s | GitHub API rate limit: set `SCC_GITHUB_TOKEN`, or pass `--version` to skip the API lookup |
| `scc index` reports staleness after edits | Expected: re-run `scc index` (incremental) or `scc verify` before trusting context |
| Wrong tool runs | You installed `scc` from npm/brew/crates.io — see "Package names" above |

<!-- trace:v1 id=doc.scc-install-maintainers type=document work=WORK-SCC-DISTRIBUTION -->
## For maintainers

- `scripts/package_release.sh <version> <outdir> [binary] [sbom-source]` is the
  single source of truth for release asset naming (binary, SBOM, installer,
  checksum file). `.github/workflows/release.yml` calls it, so a rename happens
  in one place.
- `scripts/install_contract_test.sh` builds a fixture release with that script
  and drives the installer against it offline: valid fixture, tampered payload,
  missing checksum entry, a binary that cannot run, an unsupported platform, and
  the token path (no argv exposure, no token on downloads). CI runs it in the
  `install-smoke` job, next to a canary that installs the published release —
  the contract test catches a rename in the release workflow, the canary catches
  one that only reached the published assets.

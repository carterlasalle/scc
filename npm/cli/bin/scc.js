#!/usr/bin/env node
// Launcher for the `scc` binary.
//
// The binary itself ships in a platform package (`@carterlasalle/scc-linux-x64`,
// `@carterlasalle/scc-darwin-arm64`) declared as an optional dependency, so npm
// installs only the one that matches the host. This shim finds it, makes sure
// it is executable, and hands over argv/stdio/exit status unchanged — so
// `npx @carterlasalle/scc mcp`, `scc context task "..."` and everything else
// behave exactly like a native install.
//
// Override with SCC_BIN=/path/to/scc to run a locally built binary instead.

const fs = require("node:fs");
const path = require("node:path");
const { spawn } = require("node:child_process");

// trace:exempt reason=const-data
const PLATFORM_PACKAGES = {
  "linux-x64": "@carterlasalle/scc-linux-x64",
  "darwin-arm64": "@carterlasalle/scc-darwin-arm64",
};

// trace:exempt reason=const-data
const INSTALL_HINT = [
  "Install it another way:",
  "  curl -fsSL https://raw.githubusercontent.com/carterlasalle/scc/main/scripts/install.sh | sh",
  "  or: cargo install scc-cli",
  "See https://github.com/carterlasalle/scc/blob/main/docs/INSTALL.md",
].join("\n");

// trace:exempt reason=internal-helper
function fail(message, code) {
  process.stderr.write(`scc: ${message}\n${INSTALL_HINT}\n`);
  process.exit(code);
}

// trace:exempt reason=internal-helper
function resolveBinary() {
  const override = process.env.SCC_BIN;
  if (override) {
    return override;
  }

  const key = `${process.platform}-${process.arch}`;
  const pkg = PLATFORM_PACKAGES[key];
  if (!pkg) {
    fail(
      `no prebuilt binary for ${key}. Published platforms: linux-x64, darwin-arm64.`,
      3,
    );
  }

  let packageDir;
  try {
    packageDir = path.dirname(require.resolve(`${pkg}/package.json`));
  } catch {
    fail(
      `${pkg} is not installed (it is an optional dependency, skipped on other platforms).`,
      5,
    );
  }
  return path.join(packageDir, "bin", "scc");
}

// trace:v1 id=impl.npm.launcher work=WORK-SCC-DISTRIBUTION title="npm launcher: resolve the platform binary and exec it"
// trace:exempt reason=internal-helper
function main() {
  const binary = resolveBinary();

  if (!fs.existsSync(binary)) {
    fail(`expected the binary at ${binary} but it is not there.`, 5);
  }

  // npm preserves file modes, but a tarball unpacked by another tool may not.
  // The install is user-writable, so make it executable rather than failing.
  try {
    fs.accessSync(binary, fs.constants.X_OK);
  } catch {
    try {
      fs.chmodSync(binary, 0o755);
    } catch (error) {
      fail(`cannot make ${binary} executable: ${error.message}`, 5);
    }
  }

  const child = spawn(binary, process.argv.slice(2), { stdio: "inherit" });
  child.on("error", (error) => {
    fail(`failed to run ${binary}: ${error.message}`, 5);
  });
  child.on("exit", (code, signal) => {
    if (signal) {
      process.kill(process.pid, signal);
      return;
    }
    process.exit(code === null ? 1 : code);
  });
}

main();

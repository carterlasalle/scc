#!/usr/bin/env node
// Contract test for the npm launcher (npm/cli/bin/scc.js).
//
// Builds a fixture install — the launcher plus a platform package whose binary
// is a stub — and checks the four things the published packages depend on:
//
//   1. the launcher finds the platform package through node_modules resolution
//   2. argv is forwarded unchanged
//   3. the child's exit code (and a nonzero one) is forwarded
//   4. a missing platform package fails with exit 5 and an actionable hint
//   5. SCC_BIN overrides the platform package (local builds)
//
// Run: node npm/test-launcher.js     (exit 0 = pass)

"use strict";

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { spawnSync } = require("node:child_process");

// trace:exempt reason=const-data
const repo = path.resolve(__dirname, "..");
const launcherSource = path.join(repo, "npm", "cli", "bin", "scc.js");
// trace:exempt reason=const-data
const platformName = `@carterlasalle/scc-${process.platform === "darwin" ? "darwin-arm64" : "linux-x64"}`;

// trace:exempt reason=const-data
let failures = 0;
// trace:exempt reason=internal-helper
function check(name, condition, detail) {
  if (condition) {
    console.log(`ok  ${name}`);
  } else {
    failures += 1;
    console.error(`FAIL: ${name}${detail ? `\n      ${detail}` : ""}`);
  }
}

// trace:exempt reason=const-data
const work = fs.mkdtempSync(path.join(os.tmpdir(), "scc-npm-"));
process.on("exit", () => fs.rmSync(work, { recursive: true, force: true }));

// trace:exempt reason=internal-helper
function makeStub(target, body) {
  fs.mkdirSync(path.dirname(target), { recursive: true });
  fs.writeFileSync(target, `#!/usr/bin/env node\n${body}\n`);
  fs.chmodSync(target, 0o755);
}

// A fixture install: node_modules/<launcher> + node_modules/<platform package>,
// exactly the layout npm produces for a global install.
// trace:exempt reason=internal-helper
function installFixture(dir, { withPlatform = true } = {}) {
  const launcherDir = path.join(dir, "node_modules", "@carterlasalle", "scc", "bin");
  fs.mkdirSync(launcherDir, { recursive: true });
  fs.copyFileSync(launcherSource, path.join(launcherDir, "scc.js"));

  if (withPlatform) {
    const platformDir = path.join(dir, "node_modules", ...platformName.split("/"));
    fs.mkdirSync(platformDir, { recursive: true });
    fs.writeFileSync(
      path.join(platformDir, "package.json"),
      JSON.stringify({ name: platformName, version: "0.0.0-test", bin: { scc: "bin/scc" } }),
    );
    // Echoes argv so the test can assert forwarding, and mirrors an env-provided
    // exit code so the test can assert status forwarding.
    makeStub(
      path.join(platformDir, "bin", "scc"),
      [
        'const args = process.argv.slice(2);',
        'process.stdout.write(`stub-scc ${args.join(" ")}`.trim() + "\\n");',
        "process.exit(Number(process.env.STUB_EXIT || 0));",
      ].join("\n"),
    );
  }
  return path.join(launcherDir, "scc.js");
}

// trace:exempt reason=internal-helper
function run(script, args, env = {}) {
  return spawnSync(process.execPath, [script, ...args], {
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

// trace:v1 id=test.npm.launcher-contract work=WORK-SCC-DISTRIBUTION title="npm launcher contract test"
// trace:exempt reason=internal-helper
function testArgvAndExitForwarding() {
  const dir = path.join(work, "ok");
  const script = installFixture(dir);

  const help = run(script, ["context", "task", "add retry"]);
  check(
    "launcher resolves the platform package and forwards argv",
    help.status === 0 && help.stdout.trim() === "stub-scc context task add retry",
    `status=${help.status} stdout=${JSON.stringify(help.stdout)} stderr=${help.stderr}`,
  );

  const failed = run(script, ["--nope"], { STUB_EXIT: "42" });
  check(
    "launcher forwards a nonzero exit code",
    failed.status === 42,
    `expected 42, got ${failed.status}`,
  );
}

// trace:exempt reason=internal-helper
function testMissingPlatformPackage() {
  const dir = path.join(work, "missing");
  const script = installFixture(dir, { withPlatform: false });
  const result = run(script, ["--version"]);
  const combined = `${result.stdout}${result.stderr}`;
  check(
    "missing platform package exits 5 with an install hint",
    result.status === 5 &&
      combined.includes("optional dependency") &&
      combined.includes("scripts/install.sh"),
    `status=${result.status} output=${JSON.stringify(combined)}`,
  );
}

// trace:exempt reason=internal-helper
function testSccBinOverride() {
  const dir = path.join(work, "override");
  const script = installFixture(dir, { withPlatform: false });
  const localBin = path.join(work, "local-scc");
  makeStub(localBin, 'process.stdout.write("local-build\\n");');
  const result = run(script, ["--version"], { SCC_BIN: localBin });
  check(
    "SCC_BIN runs a locally built binary",
    result.status === 0 && result.stdout.trim() === "local-build",
    `status=${result.status} stdout=${JSON.stringify(result.stdout)}`,
  );
}

testArgvAndExitForwarding();
testMissingPlatformPackage();
testSccBinOverride();

console.log(failures === 0 ? "\nnpm launcher contract: PASS" : `\n${failures} check(s) failed`);
process.exit(failures === 0 ? 0 : 1);

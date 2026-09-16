// SCC update notifier — shared checker logic for session-startup adapters.
//
// Stale-while-revalidate: session startup only reads a tiny local cache
// (~/.cache/scc/update.json) and semver-compares against the installed CLI.
// Network (GitHub releases API) happens ONLY as a detached background
// refresh, never on the startup path. Failures are silent by design — a
// missed update reminder must never break or slow a session.
import { get } from "node:https";
import { appendFileSync, mkdirSync, readFileSync, statSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

// trace:exempt reason=const-data
const REPO = "carterlasalle/scc";
// trace:exempt reason=const-data
const CACHE_FILE = join(homedir(), ".cache", "scc", "update.json");
// trace:exempt reason=const-data
const REFRESH_AFTER_MS = 12 * 60 * 60 * 1000;
// trace:exempt reason=const-data
const RENOTIFY_AFTER_MS = 24 * 60 * 60 * 1000;
// trace:exempt reason=const-data
const HTTP_TIMEOUT_MS = 6000;

type Cache = {
  latest: string;
  checkedAt: number;
  lastNotifiedVersion?: string;
  lastNotifiedAt?: number;
};

// trace:exempt reason=internal-detail
const readCache = (): Cache | undefined => {
  try {
    const raw = readFileSync(CACHE_FILE, "utf8");
    const c = JSON.parse(raw) as Partial<Cache>;
    if (typeof c.latest !== "string" || typeof c.checkedAt !== "number") return undefined;
    return c as Cache;
  } catch {
    return undefined;
  }
};

// trace:exempt reason=internal-detail
const writeCache = (c: Cache): void => {
  try {
    mkdirSync(join(homedir(), ".cache", "scc"), { recursive: true });
    writeFileSync(CACHE_FILE, JSON.stringify(c));
  } catch {
    // Cache is best-effort; a read-only HOME must not break sessions.
  }
};

// Numeric semver compare on the leading x.y.z; pre-release suffixes are
// ignored (a `-rc` and its release compare equal — good enough for a
// reminder, and it keeps this dependency-free).
// trace:exempt reason=internal-detail
const cmpSemver = (a: string, b: string): number => {
  const pa = a.replace(/^[v=\s]+/, "").split("-")[0].split(".").map(Number);
  const pb = b.replace(/^[v=\s]+/, "").split("-")[0].split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (d !== 0 || Number.isNaN(d)) return Number.isNaN(d) ? 0 : d;
  }
  return 0;
};

// Detached refresh: latest GitHub release tag -> cache. Never awaited by
// callers; never throws (all failures resolve silently).
// trace:exempt reason=internal-detail
const refreshInBackground = (): void => {
  try {
    const req = get(
      `https://api.github.com/repos/${REPO}/releases/latest`,
      { headers: { "User-Agent": "scc-update-check", Accept: "application/vnd.github+json" } },
      (res) => {
        let body = "";
        res.on("data", (c: unknown) => {
          body += String(c);
        });
        res.on("end", () => {
          try {
            const tag = (JSON.parse(body) as { tag_name?: unknown }).tag_name;
            if (typeof tag !== "string" || !tag) return;
            const prev = readCache();
            writeCache({
              latest: tag,
              checkedAt: Date.now(),
              lastNotifiedVersion: prev?.lastNotifiedVersion,
              lastNotifiedAt: prev?.lastNotifiedAt,
            });
          } catch {
            // Malformed API response — keep the old cache.
          }
        });
      },
    );
    req.setTimeout(HTTP_TIMEOUT_MS, () => req.destroy());
    req.on("error", () => {});
  } catch {
    // Offline / DNS / sandbox — the reminder simply stays quiet.
  }
};

// Cached check: returns a human-readable update notice, or undefined when
// installed is current, the cache is empty/fresh-miss, or this version was
// already notified within RENOTIFY_AFTER_MS. Kicks a background refresh
// when the cache is older than REFRESH_AFTER_MS. Pure cache I/O + string
// compare on the call path — no subprocess, no network.
// trace:v1 id=ops.scc.update-check work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export const checkCachedUpdate = (installed: string): string | undefined => {
  const now = Date.now();
  const cache = readCache();
  if (!cache || now - cache.checkedAt > REFRESH_AFTER_MS) {
    refreshInBackground();
  }
  if (!cache) return undefined;
  if (cmpSemver(cache.latest, installed) <= 0) return undefined;
  if (
    cache.lastNotifiedVersion === cache.latest &&
    typeof cache.lastNotifiedAt === "number" &&
    now - cache.lastNotifiedAt < RENOTIFY_AFTER_MS
  ) {
    return undefined;
  }
  writeCache({ ...cache, lastNotifiedVersion: cache.latest, lastNotifiedAt: now });
  const clean = (v: string): string => v.replace(/^[v=\s]+/, "");
  return `SCC ${clean(installed)} is outdated — ${clean(cache.latest)} available. Update the CLI and re-run \`scc setup omp\` to refresh the extension.`;
};

// Extension event log: every spawn (command, duration, outcome), every
// busy-skip, every update notice — one JSONL line each, with the repo dir
// on every line so per-repo failures and timings are greppable from the
// single file. Best-effort and silent: logging must never break a session.
// Rotated by truncation (last 2000 lines) past 1MB.
// trace:v1 id=ops.scc.extension-log work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export const logEvent = (repo: string, event: string, detail: Record<string, unknown>): void => {
  try {
    const file = join(homedir(), ".cache", "scc", "extension.log");
    mkdirSync(join(homedir(), ".cache", "scc"), { recursive: true });
    appendFileSync(file, JSON.stringify({ ts: Date.now(), repo, event, ...detail }) + "\n");
    try {
      if (statSync(file).size > 1000000) {
        const lines = readFileSync(file, "utf8").split("\n").filter(Boolean);
        writeFileSync(file, lines.slice(-2000).join("\n") + "\n");
      }
    } catch {
      // rotation is cosmetic
    }
  } catch {
    // a read-only HOME must not break sessions
  }
};

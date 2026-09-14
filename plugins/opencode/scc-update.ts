// SCC update reminder (OpenCode project plugin): stale-while-revalidate.
// Installed by `scc setup opencode` into `.opencode/plugins/`. On
// `session.created` it reads the local cache only, shows a TUI toast when
// a newer release is due, and kicks a detached background refresh when the
// cache is stale. Never blocks, never fails loudly.
import { readFileSync, writeFileSync, mkdirSync, existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { get } from "node:https";
import type { Plugin } from "@opencode-ai/plugin";

// trace:exempt reason=const-data
const REPO = "carterlasalle/system_ir";
// trace:exempt reason=const-data
const REFRESH_AFTER_MS = 12 * 60 * 60 * 1000;
// trace:exempt reason=const-data
const RENOTIFY_AFTER_MS = 24 * 60 * 60 * 1000;

// trace:exempt reason=internal-detail
type Cache = {
  latest: string;
  checkedAt: number;
  lastNotifiedVersion?: string;
  lastNotifiedAt?: number;
};

// trace:exempt reason=internal-detail
const cacheFile = (): string => join(homedir(), ".cache", "scc", "update.json");

// trace:exempt reason=internal-detail
const readCache = (): Cache | undefined => {
  try {
    const c = JSON.parse(readFileSync(cacheFile(), "utf8")) as Partial<Cache>;
    if (typeof c.latest !== "string" || typeof c.checkedAt !== "number") return undefined;
    return c as Cache;
  } catch {
    return undefined;
  }
};

// trace:exempt reason=internal-detail
const cmpSemver = (a: string, b: string): number => {
  const pa = a.replace(/^[v=\s]+/, "").split("-")[0].split(".").map(Number);
  const pb = b.replace(/^[v=\s]+/, "").split("-")[0].split(".").map(Number);
  for (let i = 0; i < 3; i++) {
    const d = (pa[i] || 0) - (pb[i] || 0);
    if (!Number.isNaN(d) && d !== 0) return d;
  }
  return 0;
};

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
            mkdirSync(join(homedir(), ".cache", "scc"), { recursive: true });
            writeFileSync(
              cacheFile(),
              JSON.stringify({
                latest: tag,
                checkedAt: Date.now(),
                lastNotifiedVersion: prev?.lastNotifiedVersion,
                lastNotifiedAt: prev?.lastNotifiedAt,
              }),
            );
          } catch {
            // keep the old cache
          }
        });
      },
    );
    req.setTimeout(6000, () => req.destroy());
    req.on("error", () => {});
  } catch {
    // offline — stay quiet
  }
};

// trace:v1 id=ops.scc.opencode-update work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
export const SccUpdatePlugin: Plugin = async ({ client, $ }) => {
  return {
    event: async ({ event }) => {
      if (event.type !== "session.created") return;
      try {
        const out = await $`scc --version`.text();
        const installed = out.trim().split(/\s+/)[1] || "";
        if (!installed) return;
        const now = Date.now();
        const cache = readCache();
        if (!cache || now - cache.checkedAt > REFRESH_AFTER_MS) refreshInBackground();
        if (!cache) return;
        if (cmpSemver(cache.latest, installed) <= 0) return;
        if (
          cache.lastNotifiedVersion === cache.latest &&
          typeof cache.lastNotifiedAt === "number" &&
          now - cache.lastNotifiedAt < RENOTIFY_AFTER_MS
        ) {
          return;
        }
        try {
          mkdirSync(join(homedir(), ".cache", "scc"), { recursive: true });
          writeFileSync(
            cacheFile(),
            JSON.stringify({ ...cache, lastNotifiedVersion: cache.latest, lastNotifiedAt: now }),
          );
        } catch {
          // best-effort
        }
        // trace:exempt reason=internal-detail
        const clean = (v: string): string => v.replace(/^[v=\s]+/, "");
        await client.tui.showToast({
          body: {
            title: "SCC update",
            message: `SCC ${clean(installed)} is outdated - ${clean(cache.latest)} available. Update the CLI.`,
            variant: "warning",
            duration: 8000,
          },
        });
      } catch {
        // never break session creation
      }
    },
  };
};

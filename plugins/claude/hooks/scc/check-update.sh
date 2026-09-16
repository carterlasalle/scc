#!/usr/bin/env bash
# trace:v1 id=ops.scc.claude-check-update work=WORK-SI-MMMJA4G6 satisfies=REQ-SI-503JSBGP
# SCC update check (Claude SessionStart): stale-while-revalidate reminder.
# Cache-only on the startup path; detached background refresh when the
# cache is missing or older than 12h. Prints {"systemMessage": "..."}
# when a newer release is due, otherwise prints nothing. Never fails
# loudly: every error path exits 0 silently.
SCC_BIN="${SCC_BIN:-scc}"
command -v "$SCC_BIN" >/dev/null 2>&1 || exit 0
command -v python3 >/dev/null 2>&1 || exit 0
INSTALLED="$("$SCC_BIN" --version 2>/dev/null | awk '{print $2}')"
[ -n "$INSTALLED" ] || exit 0
export SCC_UPDATE_CACHE="${HOME}/.cache/scc/update.json"
export SCC_UPDATE_INSTALLED="$INSTALLED"
python3 - <<'PYEOF2' 2>/dev/null || exit 0
import json, os, subprocess, sys, time
CACHE = os.environ["SCC_UPDATE_CACHE"]
INSTALLED = os.environ["SCC_UPDATE_INSTALLED"]
REPO = "carterlasalle/scc"
REFRESH_AFTER_MS = 12 * 3600 * 1000
RENOTIFY_AFTER_MS = 24 * 3600 * 1000
REFRESH_SRC = (
    "import json,os,time,urllib.request\n"
    "CACHE=%r\nREPO=%r\n" % (CACHE, REPO) +
    "req=urllib.request.Request("
    "'https://api.github.com/repos/'+REPO+'/releases/latest',"
    "headers={'User-Agent':'scc-update-check',"
    "'Accept':'application/vnd.github+json'})\n"
    "tag=json.load(urllib.request.urlopen(req,timeout=6)).get('tag_name','')\n"
    "prev={}\n"
    "try:\n prev=json.load(open(CACHE))\nexcept Exception:\n pass\n"
    "prev['latest']=tag\nprev['checkedAt']=int(time.time()*1000)\n"
    "os.makedirs(os.path.dirname(CACHE),exist_ok=True)\n"
    "json.dump(prev,open(CACHE,'w'))\n"
)
def semkey(v):
    v = v.lstrip("v= ").split("-")[0]
    parts = []
    for x in v.split(".")[:3]:
        try:
            parts.append(int(x))
        except ValueError:
            parts.append(0)
    while len(parts) < 3:
        parts.append(0)
    return tuple(parts)
try:
    cache = json.load(open(CACHE))
except Exception:
    cache = {}
now = int(time.time() * 1000)
if not cache.get("checkedAt") or now - cache["checkedAt"] > REFRESH_AFTER_MS:
    try:
        subprocess.Popen([sys.executable, "-c", REFRESH_SRC],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL, start_new_session=True)
    except Exception:
        pass
latest = cache.get("latest", "")
if latest and semkey(latest) > semkey(INSTALLED):
    if not (cache.get("lastNotifiedVersion") == latest
            and now - cache.get("lastNotifiedAt", 0) < RENOTIFY_AFTER_MS):
        cache["lastNotifiedVersion"] = latest
        cache["lastNotifiedAt"] = now
        try:
            os.makedirs(os.path.dirname(CACHE), exist_ok=True)
            json.dump(cache, open(CACHE, "w"))
        except Exception:
            pass
        clean = lambda v: v.lstrip("v= ")
        print(json.dumps({"systemMessage":
            "SCC %s is outdated - %s available. Update the CLI to refresh context quality."
            % (clean(INSTALLED), clean(latest))}))
PYEOF2
exit 0

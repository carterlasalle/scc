# Deployment and Infrastructure
<!-- trace:v1 id=REQ-SCC-DEPLOY type=requirement derived_from=PRD-SCC-001 title="Deployment modes, Docker, observability" -->

## 1. Modes

### Local developer
`scc serve` (the daemon lives in the `scc` binary) + SQLite + local extractors +
loopback HTTP/MCP.

### CI
```bash
scc index --ci
scc verify
scc drift
scc impact --diff origin/main...HEAD
```

### Team server
Post-MVP: API, repo workers, queue, Postgres, object storage, auth, optional graph/vector services.

## 2. Local daemon

`scc serve`: loopback port (`security.listen`, default `127.0.0.1:7777`), per-repo
DB, watcher, bounded worker pool. There is no separate `sccd` binary.

## 3. Docker

Published image: `ghcr.io/carterlasalle/scc` (`:latest`, plus `:vX.Y.Z` per
release). Read-only repo mount + writable SCC data volume; `SCC_STATE_DIR=/data`
is set in the image, and the daemon refuses a non-loopback bind without
`SCC_ALLOW_REMOTE_LISTEN=1`. Working recipes:
[docs/INSTALL.md](INSTALL.md#docker).

## 4. Resource targets

For 250k LOC baseline:
- < 2 GB memory
- index storage < 10% source size excluding optional embeddings/traces

## 5. Observability

OpenTelemetry for indexing, extractors, fact counts, staleness, context latency, tokens, cache, errors. Never export source content by default.

## 6. CI cache

Key by repository tree + SCC schema/extractor versions.

## 7. Releases

Static binaries where possible, container image, thin JS integration packages, signed checksums.

## 8. Migrations

Transactional DB migrations with fixture tests and version metadata.

## 9. Scaling

Do not prematurely build distributed team-server architecture for MVP.

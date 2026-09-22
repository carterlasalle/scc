# trace:v1 id=ops.scc-image work=WORK-SCC-DISTRIBUTION title="SCC daemon image: read-only repo mount, writable state volume"
# SCC daemon image (docs/DEPLOYMENT_AND_INFRA.md §3): read-only repo mount
# at /repo, writable state at /data (mount an SCC volume).
FROM rust:1.97-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# scc-cli embeds the harness integrations with include_str! (plugins/omp/scc,
# plugins/hermes/scc, plugins/claude/hooks, plugins/opencode), so the build needs
# plugins/ as well as crates/. scripts/docker_context_test.sh asserts that every
# include_str! path is covered here, and CI runs it — the image build failed on
# exactly this before.
COPY plugins plugins
RUN cargo build --release -p scc-cli

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /build/target/release/scc /usr/local/bin/scc
# OCI labels: org.opencontainers.image.source links the published package to this
# repository on GHCR (and is what lets the registry inherit the README).
LABEL org.opencontainers.image.source="https://github.com/carterlasalle/scc" \
      org.opencontainers.image.url="https://github.com/carterlasalle/scc" \
      org.opencontainers.image.documentation="https://github.com/carterlasalle/scc/blob/main/docs/INSTALL.md" \
      org.opencontainers.image.title="System Context Compiler" \
      org.opencontainers.image.description="Compile a repository into an evidence-backed system model and emit task-specific context packs for coding agents" \
      org.opencontainers.image.licenses="MIT"
VOLUME ["/data"]
ENV SCC_STATE_DIR=/data
WORKDIR /repo
EXPOSE 7777
ENTRYPOINT ["scc"]
# `serve` runs the loopback HTTP daemon; `mcp` runs the MCP server on stdio:
#   docker run -i --rm -v "$PWD:/repo:ro" -v scc-data:/data ghcr.io/carterlasalle/scc mcp
CMD ["serve"]

<!-- trace:v1 id=doc.scc-crate-cli type=document work=WORK-SCC-DISTRIBUTION -->
# scc-cli

The System Context Compiler CLI, local daemon, MCP server, and harness
integrations. Installs the `scc` binary.

SCC compiles code, configuration, infrastructure, runtime evidence and declared
intent into a **provenance-checked model of your system**, then emits small
task-specific context packs so coding agents start with correct system
understanding instead of rediscovering it through search.

```
scc init && scc index
scc context task "add retry to the payment webhook"
scc setup claude        # or codex / opencode / hermes / omp / pi
```

MCP server (ten semantic tools) over stdio: `scc mcp`.

- Source, docs, benchmarks: <https://github.com/carterlasalle/scc>
- Install guide: <https://github.com/carterlasalle/scc/blob/main/docs/INSTALL.md>

License: MIT.

## What changed

<!-- One paragraph: the behavior change and why. Link the work item / ticket if there is one. -->

## Evidence

<!-- Commands you ran and what they returned. Paste real output; do not summarize it away. -->

```
cargo clippy --workspace -- -D warnings
cargo test --workspace
cargo run -p scc-cli --bin scc -- bench context --min-recall 0.9
trace verify --changed
```

## Checklist

- [ ] `cargo clippy --workspace -- -D warnings` is clean
- [ ] `cargo test --workspace` passes (or: the failures are named below with a reason)
- [ ] `scc bench context --min-recall 0.9` still passes if ranking, budgets, or extraction changed
- [ ] `trace verify --changed` passes under the active policy (`.trace/policy.toml`)
- [ ] Docs updated when behavior or CLI surface changed (`docs/`, `README.md`)
- [ ] No secrets, tokens, or private repository content in code, fixtures, or this description

## Behavior notes

<!--
Anything a reviewer needs to know that the diff does not show:
- provenance/authority changes (did any fact class move up or down?)
- budget or determinism changes (is output still byte-identical for the same inputs?)
- new dependencies, network calls, or filesystem/credential scope (adapter manifest)
- anything deliberately left out of scope
-->

"""SDK completeness gate (spec §90): SDKs speak the operation registry
via invoke() — they never parse rendered Markdown or synthesize structured
state (zero tokens, empty ids, synthetic packs)."""


# trace:exempt reason=unit-test

from pathlib import Path

PY = Path(__file__).parent / "scc_sdk.py"
TS = Path(__file__).parent.parent / "typescript" / "src" / "index.ts"

# Construction/synthesis patterns only — type names in interfaces and
# return-shape docs are legitimate (they describe, not synthesize).
FORBIDDEN = [
    "ContextPack(",
    ".split("##",
    ".split('#",
    "tokens: 0",
    "tokens=0",
    "entity_ids: []",
    "entity_ids=[]",
]

# trace:exempt reason=internal-detail
def _code_lines(path):
    out = []
    for i, line in enumerate(path.read_text().split("\n"), 1):
        s = line.strip()
        if not s or s[0] in "#/*" or s.startswith("//"):
            continue;
        out.append((i, line))
    return out


# trace:v1 id=test.sdk-completeness-gate work=WORK-SI-MMMJA4G6 verifies=REQ-SI-503JSBGP exercises=impl.sdk-python-scc-sdk-scc.invoke
def test_no_synthesis_patterns():
    violations = []
    for path in (PY, TS):
        for i, line in _code_lines(path):
            low = line.lower()
            if "gate" in low or "never" in low or "synthesi" in low:
                continue
            for pat in FORBIDDEN:
                if pat in line:
                    violations.append(f"{path.name}:{i}: {line.strip()[:100]}")
    assert not violations, (
        "SDK synthesizes state from text (spec §90):\n" + "\n".join(violations)
    )

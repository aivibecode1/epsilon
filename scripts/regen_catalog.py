#!/usr/bin/env python3
"""Regenerate crates/epsilon-llm/src/catalog_data.rs from models.dev.

Usage:
    python scripts/regen_catalog.py [path-to-api.json]

With no argument the script downloads https://models.dev/api.json (via urllib,
no third-party deps) to a temp file first. Stdlib only; works on any Python 3.9+.

What it writes:
    crates/epsilon-llm/src/catalog_data.rs -- a `pub(super) const CATALOG_ROWS`
    slice of ModelInfo rows, one per current-generation chat/tool model on the
    nine direct-API providers Epsilon speaks natively, plus the fixed offline
    rows (faux, ollama).

Row selection rules (kept deliberately simple and auditable):
  - provider in PROVIDERS below (the ones catalog.rs routes wires for)
  - tool_call == true            (this harness streams text + tools)
  - output modalities == ["text"] (image/TTS/embedding/veo rows omitted)
  - id does not end in -YYYYMMDD (dated snapshots duplicate the canonical row)
  - advertised context window > 0 (rows with unknown windows cannot route)
  - first provider wins on duplicate ids (groq's `openai/gpt-oss-*` vs
    openrouter's vendor-prefixed ids): direct API beats aggregator so routing
    stays unambiguous; the loser is logged under "skipped".

Prices are models.dev USD-per-MTok times 1e6, matching the existing
`input_cost_micro_per_mtok` convention exactly. Missing costs pass through as
zero (OpenRouter genuinely lists free tiers); catalog.rs's test allows zero
only where it is real.

After running:
    cargo fmt
    cargo test -p epsilon-llm        (fix moved pins in catalog_is_current_generation)
    commit catalog_data.rs + regen_catalog.py together with a dev-log note.
"""

import datetime
import json
import re
import sys
import tempfile
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
OUT = REPO / "crates" / "epsilon-llm" / "src" / "catalog" / "catalog_data.rs"
URL = "https://models.dev/api.json"

# registry name -> (catalog provider id, env key, openai-compat base url)
# anthropic/google use their own wire consts; everyone else is OpenAI-compat.
# Order matters: it is the display order AND the dup-resolution priority.
PROVIDERS = {
    "anthropic":   ("anthropic", "ANTHROPIC_API_KEY",   None),
    "openai":      ("openai",     "OPENAI_API_KEY",      "https://api.openai.com/v1"),
    "deepseek":    ("deepseek",   "DEEPSEEK_API_KEY",    "https://api.deepseek.com/v1"),
    "google":      ("google",     "GEMINI_API_KEY",      None),
    "xai":         ("xai",        "XAI_API_KEY",         "https://api.x.ai/v1"),
    "groq":        ("groq",       "GROQ_API_KEY",        "https://api.groq.com/openai/v1"),
    "moonshotai":  ("moonshot",   "MOONSHOT_API_KEY",    "https://api.moonshot.ai/v1"),
    "zai":         ("zai",        "ZAI_API_KEY",         "https://api.z.ai/api/paas/v4"),
    "openrouter":  ("openrouter", "OPENROUTER_API_KEY",  "https://openrouter.ai/api/v1"),
}

DATED_SUFFIX = re.compile(r"-20\d{6}$")

FAUX_ROW = """\
    ModelInfo {
        id: "faux-1",
        provider: "faux",
        label: "Faux (offline scripted)",
        context_window: 1_000_000,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: true,
        env_key: "",
        wire: super::FAUX_WIRE,
    },"""

OLLAMA_ROWS = """\
    // ---- ollama (local, no key; hand-curated: not in models.dev) ----------------------
    ModelInfo {
        id: "llama3.1:8b",
        provider: "ollama",
        label: "Llama 3.1 8B (local)",
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: false,
        env_key: "",
        wire: super::OLLAMA_WIRE,
    },
    ModelInfo {
        id: "qwen3:14b",
        provider: "ollama",
        label: "Qwen 3 14B (local)",
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        supports_tools: false,
        env_key: "",
        wire: super::OLLAMA_WIRE,
    },"""


def load_registry(argv):
    if len(argv) > 1:
        return json.loads(Path(argv[1]).read_text(encoding="utf-8"))
    tmp = Path(tempfile.gettempdir()) / "modelsdev.json"
    print(f"downloading {URL} -> {tmp}")
    with urllib.request.urlopen(URL, timeout=60) as resp:
        tmp.write_bytes(resp.read())
    return json.loads(tmp.read_text(encoding="utf-8"))


def micro_usd(cost, field):
    """models.dev cost dict (USD per MTok) -> integer micro-USD."""
    v = (cost or {}).get(field)
    if v is None:
        return 0
    return int(round(float(v) * 1_000_000))


def select_rows(registry):
    rows = []
    skipped = []
    seen_ids = {}
    for reg_name, (prov, _env, _base) in PROVIDERS.items():
        models = registry.get(reg_name, {}).get("models", {})
        for mid in sorted(models):
            m = models[mid]
            fq = f"{reg_name}/{mid}"
            if not m.get("tool_call"):
                skipped.append((fq, "no tool_call"))
                continue
            if m.get("modalities", {}).get("output") != ["text"]:
                skipped.append((fq, "non-text output"))
                continue
            if DATED_SUFFIX.search(mid):
                skipped.append((fq, "dated snapshot"))
                continue
            ctx = int((m.get("limit") or {}).get("context") or 0)
            out = int((m.get("limit") or {}).get("output") or 0)
            if ctx <= 0:
                skipped.append((fq, f"context={ctx}"))
                continue
            if mid in seen_ids:
                skipped.append((fq, f"dup of {seen_ids[mid]} row"))
                continue
            seen_ids[mid] = prov
            cost = m.get("cost") or {}
            rows.append({
                "prov": prov,
                "id": mid,
                "name": (m.get("name") or mid).replace("\\", "\\\\").replace('"', '\\"'),
                "ctx": ctx,
                "out": out,
                "pin": micro_usd(cost, "input"),
                "pout": micro_usd(cost, "output"),
            })
    ids = [r["id"] for r in rows]
    dupes = {i for i in ids if ids.count(i) > 1}
    if dupes:
        sys.exit(f"FATAL: duplicate ids after selection: {sorted(dupes)}")
    return rows, skipped


def rust_int(n):
    return f"{n:_}"


ENV_BY_PROVIDER = {pid: env for (_reg, (pid, env, _base)) in PROVIDERS.items()}
PROVIDER_BY_PID = {pid: reg for (reg, (pid, _env, _base)) in PROVIDERS.items()}


def wire_expr(prov):
    if prov == "anthropic":
        return "super::ANTHROPIC_WIRE"
    if prov == "google":
        return "super::GOOGLE_WIRE"
    base = PROVIDERS[PROVIDER_BY_PID[prov]][2]
    return f'super::openai_compat("{base}", Some("{ENV_BY_PROVIDER[prov]}"))'


def emit_row(r):
    return (
        "    ModelInfo {\n"
        f"        id: \"{r['id']}\",\n"
        f"        provider: \"{r['prov']}\",\n"
        f"        label: \"{r['name']}\",\n"
        f"        context_window: {rust_int(r['ctx'])},\n"
        f"        max_output_tokens: {rust_int(r['out'])},\n"
        f"        input_cost_micro_per_mtok: {rust_int(r['pin'])},\n"
        f"        output_cost_micro_per_mtok: {rust_int(r['pout'])},\n"
        "        supports_tools: true,\n"
        f"        env_key: \"{ENV_BY_PROVIDER[r['prov']]}\",\n"
        f"        wire: {wire_expr(r['prov'])},\n"
        "    },"
    )


def render(rows):
    parts = []
    cur = None
    for r in rows:
        if r["prov"] != cur:
            cur = r["prov"]
            title = {"zai": "z.ai", "xai": "x.ai"}.get(cur, cur)
            parts.append(f"    // ---- {title} " + "-" * max(2, 70 - len(title)))
        parts.append(emit_row(r))
    return "\n".join(parts)


def main():
    registry = load_registry(sys.argv)
    rows, skipped = select_rows(registry)
    prov_counts = {}
    for r in rows:
        prov_counts[r["prov"]] = prov_counts.get(r["prov"], 0) + 1

    today = datetime.date.today().isoformat()
    chunks = [
        "// GENERATED FILE - do not edit by hand.",
        "//",
        "// Regenerated by scripts/regen_catalog.py from https://models.dev/api.json",
        f"// on {today}. Selection rules + price mapping live in the script's doc",
        "// comment. Refresh: python scripts/regen_catalog.py && cargo fmt &&",
        "// cargo test -p epsilon-llm (fix moved pins in catalog_is_current_generation).",
        "//",
        "// Offline rows (faux, ollama) are emitted by the generator so that this",
        "// file plus catalog.rs's accessors form the complete bundled catalog.",
        "",
        "use super::ModelInfo;",
        "",
        "pub(super) const CATALOG_ROWS: &[super::ModelInfo] = &[",
        "    // ---- offline reference " + "-" * 51,
        FAUX_ROW,
        "",
        render(rows),
        "",
        OLLAMA_ROWS,
        "];",
        "",
    ]
    OUT.write_text("\n".join(chunks), encoding="utf-8", newline="\n")
    print(f"wrote {OUT} ({len(rows)} registry rows)")
    for p, n in prov_counts.items():
        print(f"  {p}: {n}")
    print(f"skipped {len(skipped)} rows:")
    for mid, why in skipped:
        print(f"  - {mid}: {why}")


if __name__ == "__main__":
    main()

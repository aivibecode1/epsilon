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
  - status != "deprecated"       (retired rows are DROPPED from the bundle
    and recorded in retired_ids.rs, where the picker badges them `retired`
    instead of `unavailable` — pi/opencode parity)
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
OUT_RET = REPO / "crates" / "epsilon-llm" / "src" / "catalog" / "retired_ids.rs"
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
    # M47: native Mistral rows (pi `mistralProvider`, baseUrl
    # `https://api.mistral.ai`, `MISTRAL_API_KEY`). models.dev carries them
    # under its `mistral` registry; without this entry the generator silently
    # drops the whole family (that is why the snapshot has none today).
    "mistral":     ("mistral",    "MISTRAL_API_KEY",     "https://api.mistral.ai"),
    # M48: Vertex rows (pi `googleVertexProvider`, `google-vertex` registry).
    # Only gemini-* tool-call rows ride the shared generateContent wire —
    # pi skips the registry's Claude/OpenAI MaaS rows (different streaming
    # path). `base` is unused for this family (location-scoped endpoints);
    # wire + env come from the dedicated branch in `wire_expr`.
    "google-vertex": ("google-vertex", "GOOGLE_CLOUD_API_KEY", None),
    # M49: Bedrock rows (pi `amazonBedrockProvider`, `amazon-bedrock`
    # registry). pi skips inference-profile-only ids + the two no-system /
    # no-streaming-tool families; `base` is unused (region-scoped endpoints
    # resolve per call) — wire + env from the dedicated branch.
    "amazon-bedrock": ("amazon-bedrock", "AWS_BEARER_TOKEN_BEDROCK", None),
    # M55: GitHub Copilot subscription rows (pi `githubCopilotProvider`). One
    # host re-serves several families; the per-row wire comes from the
    # dedicated branch in `wire_expr`. `base` is the individual-account default
    # (the credential's `proxy-ep`/enterprise domain can move it per request).
    "github-copilot": (
        "github-copilot",
        "COPILOT_GITHUB_TOKEN",
        "https://api.individual.githubcopilot.com",
    ),
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
        cache_read_cost_micro_per_mtok: 0,
        cache_write_cost_micro_per_mtok: 0,
        supports_tools: true,
        env_key: "",
        wire: super::FAUX_WIRE,
        reasoning_shape: super::ReasoningShape::StandardEffort,
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
        cache_read_cost_micro_per_mtok: 0,
        cache_write_cost_micro_per_mtok: 0,
        supports_tools: false,
        env_key: "",
        wire: super::OLLAMA_WIRE,
        reasoning_shape: super::ReasoningShape::StandardEffort,
    },
    ModelInfo {
        id: "qwen3:14b",
        provider: "ollama",
        label: "Qwen 3 14B (local)",
        context_window: 131_072,
        max_output_tokens: 8_192,
        input_cost_micro_per_mtok: 0,
        output_cost_micro_per_mtok: 0,
        cache_read_cost_micro_per_mtok: 0,
        cache_write_cost_micro_per_mtok: 0,
        supports_tools: false,
        env_key: "",
        wire: super::OLLAMA_WIRE,
        reasoning_shape: super::ReasoningShape::StandardEffort,
    },"""


def load_registry(argv):
    if len(argv) > 1:
        try:
            return json.loads(Path(argv[1]).read_text(encoding="utf-8"))
        except (OSError, ValueError) as e:
            sys.exit(f"FATAL: could not read registry fixture: {e}")
    tmp = Path(tempfile.gettempdir()) / "modelsdev.json"
    # A fetch that fails here is an outage, not a catalog bug: the drift
    # workflow turns this line red, and its job is to be readable by someone
    # who then decides whether models.dev or we are wrong. The write in main()
    # is past every parse, so a failure leaves the committed file untouched.
    try:
        print(f"downloading {URL} -> {tmp}")
        with urllib.request.urlopen(URL, timeout=60) as resp:
            tmp.write_bytes(resp.read())
        return json.loads(tmp.read_text(encoding="utf-8"))
    except (urllib.error.URLError, OSError, ValueError) as e:
        sys.exit(f"FATAL: could not fetch {URL}: {e} (committed catalog untouched)")


def micro_usd(cost, field):
    """models.dev cost dict (USD per MTok) -> integer micro-USD."""
    v = (cost or {}).get(field)
    if v is None:
        return 0
    return int(round(float(v) * 1_000_000))


def select_rows(registry):
    rows = []
    skipped = []
    retired = []
    seen_ids = {}
    for reg_name, (prov, _env, _base) in PROVIDERS.items():
        models = registry.get(reg_name, {}).get("models", {})
        for mid in sorted(models):
            m = models[mid]
            fq = f"{reg_name}/{mid}"
            if not m.get("tool_call"):
                skipped.append((fq, "no tool_call"))
                continue
            # Deprecated rows leave the bundle (pi/opencode parity: a retired
            # model is not offered as current) but stay named in
            # retired_ids.rs, so a saved id that aged out badges `retired`
            # rather than reading as never-known.
            deprecated = str(m.get("status") or "").strip().lower() == "deprecated"
            # M48: the google-vertex registry carries Claude/OpenAI MaaS rows
            # that do NOT ride the generateContent path (pi filters to
            # gemini-* for the same reason) — plus one preview pi excludes.
            if prov == "google-vertex" and (
                not mid.startswith("gemini-") or mid == "gemini-3.1-flash-lite-preview"
            ):
                skipped.append((fq, "non-gemini MaaS row"))
                continue
            # M49: pi skips Bedrock inference-profile-only ids (served via
            # profiles, not direct ids) + the two families that can't ride
            # Converse streaming tools (jamba: no streaming tool-use;
            # mistral-7b-instruct: no system messages).
            if prov == "amazon-bedrock" and (
                mid == "anthropic.claude-opus-5"
                or mid.startswith("ai21.jamba")
                or mid.startswith("mistral.mistral-7b-instruct-v0")
            ):
                skipped.append((fq, "pi-excluded bedrock family"))
                continue
            # M55: copilot ids collide with the native providers' (`claude-opus-5`,
            # `gpt-5.4`, …) and the first-provider-wins dedupe would drop every
            # copilot row, so they carry the registry prefix the vertex/bedrock
            # rows already use. `*-picker` ids are model-picker aliases (pi
            # filters them through the account's `availableModelIds`), not
            # servable ids.
            if prov == "github-copilot" and mid.endswith("-picker"):
                skipped.append((fq, "model-picker alias"))
                continue
            # M55 (absorbs M49's hand rows): the registry lists every Bedrock
            # model region- AND version-qualified
            # (`us.anthropic.claude-sonnet-4-5-20250929-v1:0`) — that is five
            # near-identical rows per model, and the version tail slips dated
            # snapshots past the `-YYYYMMDD$` rule. Our flat id space uses the
            # canonical form (`anthropic.claude-sonnet-4-5`), and region routing
            # comes from AWS_REGION/env/ARN (`BedrockWire::default_region_for`),
            # never from the row id.
            if prov == "amazon-bedrock":
                mid = re.sub(r"^(?:us|eu|au|jp|ca|uk|global)\.", "", mid)
                mid = re.sub(r"-20\d{6}(?:-v\d+:\d+)?$", "", mid)
                if not mid or mid.endswith("-") or mid.endswith("."):
                    skipped.append((fq, "unusable bedrock id"))
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
            # Provider-qualified ids (M48 vertex, M49 bedrock, M55 copilot):
            # these registries re-serve ids the native providers already claim
            # (`gemini-2.5-flash`, `gpt-5.4`, …) or ids that must stay unique in
            # our flat id space, so each carries its registry prefix — the
            # short form M48 chose for vertex (`vertex/`, not `google-vertex/`).
            prefix = {
                "google-vertex": "vertex",
                "amazon-bedrock": "bedrock",
                "github-copilot": "github-copilot",
            }.get(prov)
            row_id = f"{prefix}/{mid}" if prefix else mid
            if deprecated:
                skipped.append((fq, "deprecated (retired)"))
                retired.append(row_id)
                continue
            if row_id in seen_ids:
                skipped.append((fq, f"dup of {seen_ids[row_id]} row"))
                continue
            seen_ids[row_id] = prov
            cost = m.get("cost") or {}
            rows.append({
                "prov": prov,
                "id": row_id,
                "name": (m.get("name") or mid).replace("\\", "\\\\").replace('"', '\\"'),
                "ctx": ctx,
                "out": out,
                "pin": micro_usd(cost, "input"),
                "pout": micro_usd(cost, "output"),
                # Cache pricing: a prompt token served from cache costs a fraction of the
                # input price on every provider that publishes one, and a token WRITTEN to
                # cache usually costs more than a plain input token. Without these the cost
                # estimate prices a cache-heavy session at zero for the cached share, which
                # is why they are here (G18).
                "pcin": micro_usd(cost, "cache_read"),
                "pcw": micro_usd(cost, "cache_write"),
            })
    ids = [r["id"] for r in rows]
    dupes = {i for i in ids if ids.count(i) > 1}
    if dupes:
        sys.exit(f"FATAL: duplicate ids after selection: {sorted(dupes)}")
    return rows, skipped, retired


def rust_int(n):
    return f"{n:_}"


# Wave D (criterion 5): reasoning-request body shape per catalog row.
#
# models.dev does not publish request shapes, so this is a curated, reviewed
# family table — families only, never per-model invention. It mirrors the
# substring branches `apply_thinking` used to carry in
# crates/epsilon-llm/src/providers/openai_compat.rs (openrouter/, deepseek*,
# glm/zai/, kimi, o-series/gpt-5) and the Mistral wire (magistral-,
# mistral-small/medium, devstral-), with two deliberate fixes decided in the
# Wave D ADR amendment: the openrouter branch keyed on an `openrouter/` id
# prefix that no catalog row carries (dead — every OpenRouter row silently
# took the default branch), so the table keys on the provider instead; the
# o-series/gpt-5 compat shape IS the default branch, so it needs no rule.
# Unlisted providers fall through to StandardEffort (today's default branch,
# byte for byte); unlisted Mistral rows fall to None (today the wire sends
# nothing there either — including `labs-devstral-*`, which the old
# `devstral-` prefix never matched).
def reasoning_shape(prov, row_id):
    bare = row_id.split("/", 1)[1] if "/" in row_id else row_id
    if prov == "openrouter":
        return "OpenRouterReasoning"
    if prov == "deepseek":
        return "StandardEffort" if bare == "deepseek-chat" else "DeepSeekThinking"
    if prov == "zai":
        return "GlmThinking" if bare.startswith("glm") else "StandardEffort"
    if prov == "moonshot":
        return "StandardEffort" if "thinking" in bare else "None"
    if prov == "mistral":
        if bare.startswith("magistral-"):
            return "MagistralPromptMode"
        if bare.startswith(("mistral-small", "mistral-medium", "devstral-")):
            return "MistralEffort"
        return "None"
    return "StandardEffort"


ENV_BY_PROVIDER = {pid: env for (_reg, (pid, env, _base)) in PROVIDERS.items()}
PROVIDER_BY_PID = {pid: reg for (reg, (pid, _env, _base)) in PROVIDERS.items()}


def wire_expr(prov, mid):
    # M55: Copilot serves `claude-*` over Anthropic Messages and everything
    # else over the OpenAI-compatible surface. pi carries a per-model `api` in
    # its generated model list; models.dev does not, so the family is derived
    # from the id — the same grouping the proxy itself uses. Revisit with a
    # live token if a model ever needs `/responses` instead.
    if prov == "github-copilot":
        base = PROVIDERS[PROVIDER_BY_PID[prov]][2]
        # Emitted ids carry the registry prefix (`github-copilot/claude-opus-5`).
        bare = mid.split("/", 1)[-1]
        anthropic = "true" if bare.startswith("claude") else "false"
        return (
            f'super::github_copilot("{base}", '
            f'Some("{ENV_BY_PROVIDER[prov]}"), {anthropic})'
        )
    if prov == "anthropic":
        return "super::ANTHROPIC_WIRE"
    if prov == "google":
        return "super::GOOGLE_WIRE"
    # M44: pi routes the `openai` provider over the Responses API
    # (`openAIProvider` uses `openAIResponsesApi()`); every other compat
    # provider stays on `/chat/completions`. The generator owns this so a
    # refresh never silently reverts the rows.
    if prov == "openai":
        base = PROVIDERS[PROVIDER_BY_PID[prov]][2]
        return f'super::openai_responses("{base}", Some("{ENV_BY_PROVIDER[prov]}"))'
    # M47: pi routes the `mistral` provider over its own Conversations wire
    # (`mistralProvider` uses `mistralConversationsApi()`): same endpoint
    # shape as compat, plus Mistral's snake_case payload, 9-char tool ids,
    # family-routed reasoning, and cached-token usage keys.
    if prov == "mistral":
        base = PROVIDERS[PROVIDER_BY_PID[prov]][2]
        return f'super::mistral_conversations("{base}", Some("{ENV_BY_PROVIDER[prov]}"))'
    # M48: Vertex shares the generateContent body with the API-key Google
    # wire (same parts walk, same stop vocabulary) — only endpoint + auth
    # differ, so rows point at the unit `GOOGLE_VERTEX_WIRE`.
    if prov == "google-vertex":
        return "super::GOOGLE_VERTEX_WIRE"
    # M49: Bedrock Converse rows ride the unit `BEDROCK_WIRE` (region is
    # per-call — ARN > env > model-id default — not per-row, so the
    # generator needs no per-model endpoint logic; pi's `getBedrockBaseUrl`
    # eu./default split lives in `default_region_for`).
    if prov == "amazon-bedrock":
        return "super::BEDROCK_WIRE"
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
        f"        cache_read_cost_micro_per_mtok: {rust_int(r['pcin'])},\n"
        f"        cache_write_cost_micro_per_mtok: {rust_int(r['pcw'])},\n"
        "        supports_tools: true,\n"
        f"        env_key: \"{ENV_BY_PROVIDER[r['prov']]}\",\n"
        f"        wire: {wire_expr(r['prov'], r['id'])},\n"
        f"        reasoning_shape: super::ReasoningShape::{reasoning_shape(r['prov'], r['id'])},\n"
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


def render_retired(retired):
    ids = sorted(set(retired))
    body = "\n".join(f'    "{i}",' for i in ids)
    return (
        "// GENERATED FILE - do not edit by hand.\n"
        "//\n"
        "// Regenerated by scripts/regen_catalog.py from https://models.dev/api.json\n"
        "// alongside catalog_data.rs: ids the registry marks deprecated are\n"
        "// dropped from the bundle and recorded here, so the picker can badge\n"
        "// a saved id that aged out as `retired` rather than `unavailable`.\n"
        "\n"
        "/// Ids dropped from the bundle as deprecated, sorted (criterion 6).\n"
        "pub const RETIRED_IDS: &[&str] = &[\n"
        f"{body}\n"
        "];\n"
    )


def main():
    registry = load_registry(sys.argv)
    rows, skipped, retired = select_rows(registry)
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
    OUT_RET.write_text(render_retired(retired), encoding="utf-8", newline="\n")
    print(f"wrote {OUT_RET} ({len(set(retired))} retired ids)")
    for p, n in prov_counts.items():
        print(f"  {p}: {n}")
    print(f"skipped {len(skipped)} rows:")
    for mid, why in skipped:
        print(f"  - {mid}: {why}")


if __name__ == "__main__":
    main()

# M40 — pi-parity: model resolution (`model-resolver.ts`)

Eighth port batch against the 2026-09-03 parity audit (session/auth/config
finding S23). `--model` / `model_id` config / env was passed through as a raw
string: only exact catalog ids worked, and a typo died with a bare "no API
key" message three layers later instead of "unknown model" at parse time.

## What landed

1. **Resolver** (`epsilon-llm/src/catalog.rs`): `resolve_model` ports pi's
   match order — exact id → canonical `provider/id` (case-insensitive,
   unambiguous) → bare id case-insensitively (ambiguous across providers ⇒
   `None`, mirroring pi's rejection) → substring over id + label preferring
   alias rows (no `-YYYYMMDD` suffix, pi's `isAlias`) over dated versions,
   then highest-sorting id. Returns `ResolvedModel { info, kind }`
   (`ModelMatchKind::Exact/ProviderQualified/CaseInsensitive/Partial`).
   `resolve_model_candidates` shares the same rules and answers WHY `None`
   happened: empty = no match, several = ambiguous — one source for picker
   "did you mean?" and CLI errors.
2. **CLI wiring** (`dispatch`): the raw flag/config/env candidate resolves
   through the catalog before anything else. Non-exact hits log
   `model "X" matched catalog entry "Y"` to diagnostics; custom
   `<name>/...` ids (M28) still bypass to the custom-provider path; unknown
   dies with the reason, ambiguous dies listing the matches.
3. **Tests**: `resolve_model_follows_pi_match_order` covers all four kinds,
   alias-over-dated, blank/unknown ⇒ `None`, and the candidates split.

## Key decisions

- Deliberately NOT ported: thinking-level validation (`resolveThinkingLevel`
  clamps) — epsilon has no thinking-level knob yet, so there's nothing to
  validate against; the resolver is the portable half, the knob comes with
  the feature. Same for `defaultModel` switching (pi's anthropic-default):
  epsilon's `DEFAULT_MODEL_ID` + auto-real fallback already answers "no
  model given" and changing the default is a behavior fork, not parity.
- `lookup_model` stays exact-only (picker grouping, auth, `for_model`
  window lookup all need exactness); fuzzy matching lives ONLY in the new
  resolver + the one CLI call site.
- Ambiguous canonical/bare ⇒ `None` (not first-hit): `resolve_model("claude")`
  still returns the best partial, but `resolve_model_candidates("claude")`
  proves ambiguity — callers choose strictness.

## Gotchas

- `ResolvedModel` first derived `PartialEq, Eq` — `ModelInfo` has `f64`-shaped
  price fields (u64 micros, but no Eq derive) so the derive failed E0369/E0277.
  Dropped to `Debug, Clone, Copy`: the test compares `kind` + `info.id`, never
  whole structs.
- `and_then(|raw| ... return Some(...) ... die(...))` tripped clippy's
  `redundant_closure` lint; a `map` rewrite broke on `die() -> !` (E0308:
  `Option<Option<String>>`); settled on `match candidate_raw.as_deref()` with
  expression arms — `die` diverges so types unify.
- Windows linker held `epsilon_llm-59b4dd66…exe` across SIX consecutive
  `cargo test -p epsilon-llm` invocations (LNK1104, no live process, dir
  writable, `cargo clean -p epsilon-llm` didn't help). `CARGO_INCREMENTAL=0`
  relinked under a fresh hash and went green — same workaround as the M35/M36
  sessions. Filed mentally as toolchain flake, not code.
- Patch-tool anchor drift: `cargo fmt` reflowed the custom-provider closure
  between edits, so two patches missed their anchors — re-read the region
  (`sed -n`) before retrying instead of guessing.

## Verification

- `cargo test -p epsilon-llm --lib`: 43 passed (incl. new resolver test).
- `cargo test -p epsilon-cli`: 10 lib + 3 integration, green.
- `cargo fmt --check`: clean. Workspace clippy: zero warnings.

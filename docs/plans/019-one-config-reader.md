# 019 — one configuration reader, and the value crosses the seam

**Ledger row:** A12. **Status:** **step 1 LANDED at M223** — a delegation rather than the inversion this plan proposed, because step 1's premise was wrong (below) — and **step 2 LANDED at M224**: both OAuth parses are now one read in `core::config::oauth_config_in`, and the value crosses as `OAuthConfig` from the two callers that had them anyway. **Steps 3—5 remain**: `custom.rs` still parses `config.toml` inside llm, so `toml` cannot leave that crate yet, and the manifest edit in step 4 must wait for step 3 — which is exactly the ordering this plan's own step 0 corrected. **Estimated execution:** steps 3—5, one milestone, warm target.

> **Step 1 was mis-shaped, and the correction is the useful part (M223).** This plan and the ledger
> row both described the duplicated `global_dir` as *two copies of one rule in crates that cannot
> see each other* — which is true of `llm → core` and **false of `core → llm`**.
> `epsilon-core/Cargo.toml` depends on `epsilon-llm`, so the higher crate can see the lower one, and
> the rule has a forced home in the lower crate: `core::config::global_dir` delegates to
> `llm::auth::global_dir`. Five lines, no new edge, no signature changes anywhere, and llm's
> provider-resolution path untouched. What this plan proposed instead — threading a `&Path` through
> seven credential wrappers, twelve CLI/TUI call sites and `catalog::resolve_api_key` /
> `custom::load_custom_providers` inside llm — was a refactor measured against a call graph nobody
> had read. **This is the seventh time in one stretch that a claim in a plan inherited a neighbour's
> shape** (A7, A11, plan 023's step 1, this, and the three ledger rows written from them); the
> countermeasure is unchanged and worked again: read the thing the claim is about before planning
> against it.

## The finding, measured

Configuration is read in four places, and two of them are the *same rule* written twice in
crates that cannot see each other:

| Site | Reads | Notes |
|---|---|---|
| `epsilon-core/src/config.rs:37` | `global_dir()` → `$HOME/.config/epsilon` | the global `config.toml`, favorites, recents, disabled skills, prompt history, model id |
| `epsilon-llm/src/auth.rs` | `global_dir()` → `$HOME/.config/epsilon` | **byte-for-byte the same function**, plus the `auth.json` credential store |
| `epsilon-llm/src/custom.rs:69` | `config_dir/config.toml` → `[provider_custom.<name>]` | its own `read_to_string` + `toml::from_str`; `epsilon-llm` carries a `toml` dependency for this alone |
| `epsilon-llm/src/oauth_login.rs:775` | `[oauth.<provider>] client_id` from the global `config.toml` | the same file, read a third time |
| `epsilon-cli/src/runtime.rs:138,256` | the project `.epsilon/config.toml` + `trust.json` | the trust store is the CLI's; the project config parse is its own |

Why this is a defect rather than untidiness: **two copies of one rule cannot disagree visibly.**
`M202`'s lesson one level up — the *constant* is not the *predicate* — and this repository's own
D-class: a path rule that changes in one place and not the other produces a config file that one
crate reads and another cannot find, with no error anywhere.

## The decision

**The reader stays above the seam, and parsed values cross as parameters.**

The obvious fix — one shared reader — is blocked by the architecture law, and it is worth stating
why rather than working around it:

- `epsilon-llm` may not depend on `epsilon-core` (the DAG runs core → llm, and AGENTS.md forbids
  library → CLI and the reverse edge that would make this easy);
- `epsilon-contracts` is **types and traits only**, with zero behaviour, so a reader cannot live
  there;
- a new crate needs an ADR and would exist to hold one function.

So the inversion is the only one that fits the law already in force, and it is the pattern this
repository already uses twice: `register_builtins_with_skills(registry, …, skills)` — *the frontend
discovers, the crate receives* — and `ToolContext::task_host` — *the crate asks, the owner
answers*.

Concretely: **`epsilon-core::config` is the only reader of `config.toml`. Everything a lower crate
needs arrives as a value.**

## Step 0 — what the code actually does (checked 2026-09-30, and it corrects steps 2 and 3)

The plan was written from the ledger's description of A12. Reading `epsilon-llm` shows the
duplication is **worse** than that description and the sequencing below was **wrong**:

```text
custom.rs:70        read_to_string(config_dir/"config.toml")   ┐ custom providers
custom.rs:74        toml::from_str(&raw)                        ┘
oauth_login.rs:799  read_to_string(config_dir/"config.toml")   ┐ OAuth client id
oauth_login.rs:800  toml::from_str(&raw)                        ┘  (client_id_from_config)
oauth_login.rs:1184 read_to_string(config_dir/"config.toml")   ┐ a SECOND OAuth read,
oauth_login.rs:1185 toml::from_str(&text)                       ┘  elsewhere in the same file
```

**Three readers of one file inside one crate**, plus the two `global_dir()` copies and `core::config`
above them — five ways to find or parse one file, where the row said four places.

Two corrections that follow:

- **Step 2's "drop `toml` from `epsilon-llm`'s manifest" cannot happen at step 2.** `toml` is used by
  the OAuth path as well, so removing the custom-provider reader leaves the dependency in place and
  the manifest edit would fail the build. **The OAuth reads must go first**; `toml` leaves the manifest
  at the end, and `cargo tree` is still the proof.
- **Step 3 pointed at a doc comment.** `:775` is documentation of the client-id rule; the real reads
  are `client_id_from_config` at `:797-800` and the second one at `:1184`. Both take a config
  directory and parse the file themselves, so both need the same treatment: take the value, not the
  directory.

**Revised order:** (1) the credential-store directory, as written; (2) **both** OAuth reads take an
already-resolved client id; (3) custom providers arrive parsed; (4) `toml` leaves the manifest, proven
by `cargo tree -p epsilon-llm -i toml`; (5) the project-config parse beside the trust hash.

This is the sixth claim this session found written from a neighbour rather than from the code (A7,
A10, A11, A20, plan 023's step 1, and this). The countermeasure is unchanged and it worked here: one
read, no code, and a wrong sequencing caught before anyone started it.

## The execution, in order

Each step is independently committable and independently verifiable; stop after any of them and the
tree is coherent.

### Step 1 — DONE at M223, and not the way it was written

**Landed:** `epsilon-core/src/config.rs` delegates to `epsilon-llm/src/auth.rs`'s `global_dir`, and
`the_global_directory_is_computed_in_exactly_one_place` walks the workspace to make a second *body*
unrepeatable. Five lines, no signature changed.

**Not done, deliberately:** the plan below proposed threading the directory through the credential
wrappers. Reading the manifests first showed `epsilon-core` depends on `epsilon-llm`, so the copy in
core could simply ask the one in llm. `llm → core` is the edge the law forbids, and it is the only
one that would have required threading — the plan had inverted the direction that already exists.
The credential wrappers keep their `*_in(config_dir, …)` variants, which remain the right shape for a
caller that wants to say *which* directory; they simply stop being the only way in.

Steps 2–5 below are unchanged and are about a different subject: **parsing one file in three places
inside `epsilon-llm`**, which is why that crate carries a `toml` dependency at all.

**Evidence delivered (and one correction to this plan's own step 1 evidence):** `grep -rn "fn
global_dir" crates/` returns **two** hits, not one — two *definitions* exist by design, because a
delegate is a definition. The property is one *computation*, so the test counts the definitions that
read `$HOME`, which is what M223 asserts. The plan's version of that check could not have passed
against the fix it asked for, which is the second time in this file that the check was written from
the description rather than from the code (the first was step 0's ordering).

### Step 2 — both OAuth reads take a value (moved ahead of `toml`'s removal)

**This step was "custom providers arrive parsed" in the first draft; the order was wrong** — `toml` is
used by the OAuth path too, so the manifest cannot shed it until these reads are gone. The real sites
are `client_id_from_config` (`oauth_login.rs:797-800`) and a second read at `:1184`; both take a
config directory and parse the file themselves, and both should instead take the already-resolved
client id.

### Step 3 — custom providers arrive parsed

1. Add to `core::config` the accessor that returns the parsed table, not one key:
   `load_table_in(config_dir, "provider_custom") -> Vec<(String, toml::Value)>` (or a typed
   `CustomProviderConfig` struct in contracts — *types only*, so allowed).
2. `epsilon-llm/src/custom.rs`: delete `load_custom_providers_in` / `load_custom_providers`. Keep
   `CustomProvider::from_config(name, base_url, env_key)` — it is already pure, and it is the seam.
3. Drop `toml` from `epsilon-llm/Cargo.toml` and remove the lockfile entry if nothing else needs it
   (`cargo tree -p epsilon-llm -i toml` is the check).
4. **Evidence:** the dependency is gone from the manifest *and* from `cargo tree`; a test that a
   config file with two custom providers still produces two rows, driven through the CLI's
   composition rather than through llm's own reader (which no longer exists).

### Step 4 — the project config's parse gets an owner

`runtime.rs:138` parses `.epsilon/config.toml` into `toml::Value` for the trust hash. That parse is
*two* facts: the file's bytes (for the hash) and its contents (for the settings). Move the parse
next to the hash in `core::config` so "what the hash covers" and "what the settings are" cannot
drift; keep `trust.json` in the CLI, where the trust *decision* lives.

## What this plan deliberately does not do

- **No new crate.** ADR-014's "brick earns existence" rule; a reader for one file does not.
- **No behaviour in contracts.** Types may move there (a `CustomProviderConfig`), the reading may
  not.
- **No config *cache*.** Every call re-reads the file today and that is fine at these call
  frequencies; a cache would add an invalidation question (`/reload` exists) that nobody has asked
  for. If a profile ever shows this mattering, that is its own measurement.
- **No change to what the files mean.** This is a refactor of *who reads*, not of *what is read*:
  the TOML tables, defaults and precedence stay exactly as they are, and the existing tests for
  them are the regression net.

## Limits

- **The step estimates are unmeasured.** They are derived from reading the call sites, not from a
  timed attempt; the first step alone touches four public functions and their callers.
- **`cargo tree` after step 2 is the only proof `toml` left the crate.** A manifest edit that
  leaves a transitive path in place would look done and not be.
- **Nothing here fixes the *number* of config surfaces** (global `config.toml`, project
  `.epsilon/config.toml`, `trust.json`, `auth.json`, the session file's settings entries). Four
  files with clear owners is fine; the defect was two crates reading one file with no owner.

# 019 — one configuration reader, and the value crosses the seam

**Ledger row:** A12. **Status:** decision made (ADR-012 amendment), execution not started.
**Estimated execution:** one milestone, ~2–3 hours with a warm target.

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

## The execution, in order

Each step is independently committable and independently verifiable; stop after any of them and the
tree is coherent.

### Step 1 — the credential store takes its path (removes the duplicate `global_dir`)

1. `epsilon-llm/src/auth.rs`: replace the private `global_dir()` with a resolved path. Add
   `auth::set_store_dir(Option<PathBuf>)`-shaped configuration **or** thread the dir through the
   four entry points (`load_credentials`, `save_credential`, `remove_credential`,
   `list_credentials`). Prefer threading: a process-global would recreate the same
   two-answers problem with an ordering hazard on top.
2. `epsilon-core/src/config.rs`: keep `global_dir()` — it becomes the single definition — and have
   the CLI pass `config::global_dir()` into the auth calls.
3. **Evidence:** a test that the two crates' resolved directory is the same string, with the
   second copy deleted so it cannot pass by construction; plus
   `grep -rn "fn global_dir" crates/` returning exactly one hit.

### Step 2 — custom providers arrive parsed

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

### Step 3 — OAuth client ids arrive as values

1. `oauth_login.rs`: replace the config read at `:775` with a parameter on the login entry point.
2. `epsilon-cli`: read `[oauth.<provider>] client_id` through `core::config` and pass it.
3. **Evidence:** the existing OAuth tests keep passing with the value injected; a test that a
   missing client id produces the same guidance sentence it produces today (the sentence is the
   product surface, and rewording it silently is its own defect).

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

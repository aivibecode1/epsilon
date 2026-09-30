# M223 — one implementation of the global directory, found by reading the dependency graph

**Ledger row A12, plan 019 step 1 — and the plan was wrong about the shape, which is the finding.**
`~/.config/epsilon` was resolved by two byte-identical functions, and the plan proposed to delete one
by threading a `&Path` through seven credential wrappers, twelve CLI/TUI call sites, and llm's
provider-resolution path. Measuring that call graph first found something better.

## The measurement that replaced the plan

Plan 019 and the ledger row both describe the duplication the same way: **"two copies of one rule in
crates that cannot see each other."** The crates are `epsilon-core/src/config.rs:37` and
`epsilon-llm/src/auth.rs:230`, byte-identical bodies. What neither had checked is which way the
dependency runs:

```text
epsilon-core/Cargo.toml   epsilon-llm = { path = "../epsilon-llm" }     <- core sees llm
epsilon-llm/Cargo.toml    (no epsilon-core)                             <- llm cannot see core
```

**The duplication is asymmetric, and only one crate can delegate.** `epsilon-llm` needs the rule
because the credential store lives there, so the rule has a *forced home in the lower crate* — the
DAG decides it, not taste. `epsilon-core`, which is higher and can see `llm`, asks.

```rust
// epsilon-core/src/config.rs
pub fn global_dir() -> Option<PathBuf> {
    epsilon_llm::auth::global_dir()
}
```

Five lines, no new edge, no signature changes anywhere, and the provider-resolution path untouched.
`grep -rn "fn global_dir" crates/` — the plan's own step-1 evidence — now returns one implementation
and one delegate.

**Why the plan reached for inversion.** Its own reasoning was sound and its premise was not: A12 is
about "two copies of one rule [that] cannot disagree visibly", and an inversion is the answer the
repository uses when the crates are genuinely unrelated. The plan tested that shape by reading the
*ADR-012 amendment* ("the credential store takes its directory as a parameter rather than resolving
it") rather than by reading the two manifests. This is the fourth time in one stretch that a claim
inherited a neighbour's shape — A7, A11, plan 023's step 1, and now this — and the countermeasure is
the same one that worked each time: read the thing the claim is about, and change nothing until the
reading is in hand.

## The test, and why it reads the source

```rust
assert_eq!(computed.len(), 1, "`~/.config/epsilon` must be *computed* in one place; …");
```

**A test on a value could not catch this defect.** Two functions that return the same answer *today*
is not the property — that is exactly the trap the ledger row names. What must be unrepeatable is a
*second body*, so the test walks the workspace's `.rs` files, finds every `global_dir` definition,
and counts the ones that **read `$HOME`**. A definition that asks another crate is a delegate and is
counted separately, which is why the first version of this test (counting definitions) failed against
the fixed code and had to be corrected: two definitions exist by design, one implementation does not.

**The control** restores the copy and names both:

```text
`~/.config/epsilon` must be *computed* in one place; computed at
["..\\epsilon-core\\src\\config.rs:51", "..\\epsilon-llm\\src\\auth.rs:235"]
(0 delegating definition(s) are fine)
```

## Verification

**Battery `m223-r2` GATE PASS** — `audit/evidence/gate-m223-r2.log`: **1,646 tests across 65
binaries** against M222's 1,645: **+1**, the duplication check. Gate shape: check-links 0 s,
check-lint-tables 0 s, fmt 3 s, clippy 34 s, test 287 s, demo 11 s, real-check 5 s. Run with
`CARGO_PROFILE_DEV_DEBUG=0` / `CARGO_PROFILE_TEST_DEBUG=0` (M208's disk knob).

The first attempt (`m223`, kept as `audit/evidence/gate-m223-FAILED-clippy.log`) failed on **clippy,
not a test**: the source-walking helper sat at module level, so a shipped library carried a function
that walks the workspace. `#[cfg(test)]` is the fix, and the lint was right — a test-only helper in a
library is a test-only helper wearing a library's clothes.

## What this does *not* do

Plan 019 steps 2–5 remain, and they are about a different thing: **`config.toml` is still parsed in
three places inside `epsilon-llm`** (`custom.rs:70`, `oauth_login.rs:797` and `:1184`), so `toml` is
still that crate's dependency for no good reason. This milestone removed a duplicated *path rule*; it
did not reduce the number of *readers of one file*. The plan's revised order — OAuth reads before the
manifest sheds `toml`, then custom providers parsed, with `cargo tree -p epsilon-llm -i toml` as the
proof — is unaffected and still correct.

The plan's step 1 is retired rather than executed: threading a directory through
`resolve_api_key` and `load_custom_providers` would have reached `providers/mod.rs` and
`fallback.rs`, inside the provider-resolution path, to fix a rule that was already one rule.

## Limits

- **The test walks the workspace**, so it is a source invariant rather than a behavioural one: it
  would not notice a second implementation written in another language, in a build script, or as a
  literal `join(".config").join("epsilon")` — which is why it names `var_os("HOME")` specifically and
  why the milestone records that a scan for other spellings is a separate question.
- **Nothing asserts the delegation is *the* right owner** beyond the dependency direction. If the DAG
  ever changed, the test would still pass (one computation) while the delegate became impossible to
  compile — a compile error, which is the right failure.
- **`epsilon-llm`'s copy is now load-bearing for a crate above it.** That is the point, but it means
  a change to llm's credential module can now break core's config path, which it could not when the
  bodies were independent.

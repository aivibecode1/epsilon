# M39 — pi-parity P0 batch 5: trust lattice — resource scan + curtailed roots

Seventh port batch against the 2026-09-03 parity audit (session/auth/config
findings S18/S19 — the trust-model P0). The trust hash gated
`.epsilon/config.toml` only: skills, extensions, prompts, and themes loaded
regardless of trust, so cloning a repo executed its third-party code before
any trust question was asked.

## What landed

1. **Resource scan** (`runtime.rs`): `trust_requiring_resources` ports pi's
   `hasTrustRequiringProjectResources` — `.epsilon/config.toml`, the
   `extensions`/`skills`/`prompts`/`themes` dirs, `SYSTEM.md`,
   `APPEND_SYSTEM.md`, plus `.agents/skills` in the workspace or any ancestor
   (pi walks to root). Returns the typed `TrustRequiringResource` list (for
   diagnostics) with `has_trust_requiring_resources` as the boolean.
2. **Curtailed roots**: `skill_roots_trusted` / `extension_roots_trusted`
   take `project_trusted`; `false` drops every workspace-local root while the
   global root always loads (pi's "untrusted ⇒ settings/extensions
   curtailed"). Plain `skill_roots`/`extension_roots` delegate with `true` —
   every existing caller keeps old behavior.
3. **Wiring**: `dispatch` computes `trusted_now` from the config-hash decision
   (`Changed` ⇒ false), threads it through `SharedRun` → `RunParams` (plan
   008's composition struct — the trust bit travels with the other build
   inputs instead of a global), and `build_runtime` gates skill discovery,
   conventional extension roots, AND project-declared `[[extensions]]` dirs
   (an untrusted config can't smuggle code in through its own declarations).
   A startup notice names the curtailment.

## Key decisions

- No global trust store yet (pi's `~/.agent/trust.json` path→bool map with
  inheritance): this batch closes the EXECUTION half (untrusted code never
  loads). The persistence half (remembering per-path decisions across runs)
  isRoz next — deliberately split, because the hash decision already answers
  "trusted now?" every run and the store only adds memory.
- `/trust` + `/reload` semantics unchanged: accepting the hash flips
  `trusted_now` next run, which restores the full roots.
- Scan covers `.agents/skills` ancestors but NOT `.epsilon/*` ancestors: pi
  checks `.pi` only at cwd (its config dir is per-project by construction).

## Gotchas

- `build_runtime` takes `RunParams`, not `SharedRun` — the first wiring
  attempt referenced `shared` inside `build_runtime` and failed 3× E0425.
  Fixed by adding `project_trusted` to `RunParams` (4 construction sites).
- Test hermeticity: the ancestor walk reaches above TMP, and this machine has
  ambient `.agents/skills` above Temp — a bare-tempdir "quiet" assertion
  FAILED. Split the tests: `.epsilon/*` kinds assert `contains` (no ancestor
  dependence); the `.agents` walk gets its own test that reads the REAL
  ancestor chain and asserts the walk agrees with it. Pi-parity over
  hermeticity, with the agreement assertion as the proof.
- One patch-tool anchor misfire rewrote `seeded_workspace` in the tools tests
  earlier this session (M38 log); same discipline applied here — `git diff`
  after every multi-edit file.

## Verification

- 3 new `runtime` tests: all-7-kinds scan, `.agents` workspace+ancestor walk
  with ambient-chain agreement, curtailed-vs-trusted roots + shim back-compat.
- `cargo test -p epsilon-cli`: 10 lib + 3 integration, green.
- `cargo fmt --check`: clean. Workspace clippy: zero warnings.

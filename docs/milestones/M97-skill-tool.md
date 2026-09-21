# M97 — the `skill` tool the contract has always promised

## What

The agent contract told the model to load a skill's body "via the `skill` tool"
(`crates/epsilon-core/assets/agent-contract.md:101-106`). No such tool existed. The catalog
printed a name and a one-line description and no path, so the model could see that a skill
existed and had no way to open one — an instruction it could not follow, in every session, for
every skill. pi publishes each skill's location and points at the read tool
(`skills.ts:355-383`); this is the first gap the fidelity audit found, because it is the one a
user notices soonest (D-1).

The fix **adds the tool** rather than editing the contract to match the code:

- `crates/epsilon-tools/src/skill.rs` — `LoadableSkill { name, description, dir }`, `SkillTool`,
  the argument schema, and a `strip_frontmatter` helper.
- `crates/epsilon-tools/descriptions/skill.txt` — the tool description, in a file like every
  other built-in so the wording is reviewable next to the others.
- `epsilon_tools::register_builtins_with_skills` — registration that takes the resolved list.
- `crates/epsilon-cli/src/main.rs` — `build_runtime` builds the loadable list from the discovered
  skills, filtered on `disable_model_invocation`, and registers the tool only when that list is
  non-empty.

The second commit does the other half of the same promise: the **catalog** now filters
`disable-model-invocation` too, so the catalog and the tool agree about what exists.

## Key decisions

- **A dedicated tool, not a path in the catalog and not a contract rewrite.** The catalog stays
  name-and-description only, so a workspace with thirty skills costs thirty lines of context
  instead of thirty file bodies. Adding `<location>` and telling the model to `read` the file
  would also have worked, but it makes the model responsible for knowing that a `SKILL.md`'s
  frontmatter is metadata rather than procedure, and it spends a general-purpose tool on a
  specific job.
- **A wrong name is answered with the real list.** The failure that costs the most is a model
  retrying names it has already guessed wrong; naming the available skills — each with its
  description — ends that loop in one round trip. The test asserts the message names both the
  guessed name and the real options, with the reason written into the assertion.
- **Frontmatter is stripped on the way out.** `name`, `description` and the invocation flag are
  harness metadata the model already has from the catalog; repeating them spends the context the
  body exists to use.
- **The caller owns discovery.** Skills are files on disk found under whatever trust policy the
  frontend applies; the tool receives an already-resolved list and never searches the filesystem.
  That keeps the trust gate in one place instead of restating it here — which is also why
  `LoadableSkill` is a separate type from `epsilon-core`'s `Skill`: this crate sits below the
  crate that owns discovery, and the direction of the dependency is the reason for two types
  rather than one shared one.
- **Model-invocation is filtered at registration, not at call time.** A skill marked
  `disable-model-invocation` is a procedure the user drives; filtering the list means the model
  cannot reach one even by guessing its name. The type the tool holds has no disabled flag at
  all, so the filter cannot be skipped by accident at a call site.
- **An empty list registers no tool.** A workspace with no skills does not carry a tool that
  could only ever fail, and the CLI's `if !loadable.is_empty()` is what keeps the prompt's tool
  block honest.
- **The list is sorted by name.** A not-found message whose order changes between calls makes a
  model's earlier reasoning about it unreliable; `names_are_reported_in_a_stable_order` pins it.
- **A name that differs only in case or padding still loads.** A model that saw `Ratatui-TUI` in
  the catalog and typed `ratatui-tui` is not making a mistake worth a round trip.
- **The catalog filters the same way as the tool.** Showing the model a name it cannot use is
  worse than hiding it: an instruction that cannot be followed invites a guess and then an
  apology. Two tests pin it — a disabled skill never appears, and a catalog with nothing loadable
  is *empty* rather than a bare `# Skills` heading.

## Gotchas

- **An unclosed frontmatter fence returned the body whole rather than truncated to nothing** —
  fixed in the same commit, while wiring the tool. `strip_frontmatter` is deliberately tolerant:
  a file that opens with `---` but never closes it is returned as-is, because a skill body that
  silently vanishes is a worse failure than one that opens with a stray line. Found while
  writing the tests, not from a report.
- **The stripping also handles a BOM** — it strips `\u{feff}` before looking for the fence, so a
  skill file written by an editor that adds one is not read as having no frontmatter at all.
- **An empty body after stripping is a failure, not a blank result.** A skill whose entire
  content is frontmatter would otherwise load as nothing and look like a successful call.
- **The tool description tells the model not to call it speculatively**, and says why: the loaded
  body stays in the context window for the rest of the session, so a skill that was not needed is
  context the work no longer has. `the_description_discourages_speculative_loads` pins that the
  published description actually carries that warning — the only place the design's assumption is
  stated to the party that has to honour it.
- **`register_builtins_with_skills` exists and the CLI does not call it.** `build_runtime`
  registers `SkillTool::new(loadable)` directly, because it applies its own trust filter while
  building the list. The helper is written for the frontends that do not, and nothing in the tree
  uses it — a reader wiring a second frontend should use it, and a reader looking for the
  production path should not expect to find it there.
- **`SkillTool::names()` has no production caller.** It exists, and the stable-order test uses it;
  nothing in the CLI prints the loadable list, so the filtered catalog and the user-facing
  `/skills` view have no shared source to drift from or agree with (the catalog is filtered on
  purpose, and the user-facing list should not be — but nothing states that in code).

## Verification

No battery log for this slice is preserved, and no gate log exists for M97 (`audit/evidence/`
holds `gate-m93b`, `gate-m94c`, `gate-m95b` and `m101-gates.log`; none of them is these bytes).
The evidence is the two commit messages' own measurements, reproduced here as they state them.

From `2d50813` (the tool):

- `cargo test -p epsilon-tools` — **8 new tests, all green**; the commit does not state a total
  for the crate.
- `cargo test -p epsilon-tools -p epsilon-tui -p epsilon-cli` — **0 failed**.
- `cargo clippy` on the three crates with `-D warnings` — exit 0.
- `cargo fmt --all -- --check` — exit 0.

The eight tests are: a known skill returns its body without the frontmatter; an unknown name is
answered with the real list; a name differing only in case or padding still loads; an empty name
is rejected with guidance; an empty body is a failure rather than a silent blank; an unclosed
frontmatter block is returned whole; names are reported in a stable order; and
`the_description_discourages_speculative_loads`.

From `87c5628` (the catalog):

- `cargo test -p epsilon-core --lib skills` — **5 passed**.
- `cargo clippy` — clean; `cargo fmt --all -- --check` — exit 0.

`audit/11` records the corresponding gaps as **G1** (closed in `2d50813`) and **G6** (closed in
`87c5628`), both against the fidelity audit's D-1/D-4. A later note in
`agents/memory/port-revalidation.md` confirms the audit's D-1 row had gone stale — the tool
exists, and the filter on the tool's input type means it cannot be bypassed.

## Limits

- **The contract text was not touched and is now merely true.** `agent-contract.md:101-106` still
  names the `skill` tool; the tool exists, so the instruction is valid. Nothing pins the two
  together, so a future rename of the tool would silently invalidate the contract sentence again
  — the same class of drift as the two different context-file candidate lists (M101).
- **No size bound on a loaded body.** `read` caps a page at 2000 lines / 256 KiB; the skill tool
  reads `SKILL.md` whole and returns it. A very large skill costs whatever it costs, and the
  description's only defence is asking the model not to load speculatively.
- **Only one skill per call, and nothing tracks what is already loaded.** Loading the same skill
  twice costs its body twice; the tool has no per-session memory.
- **The tool reads from a directory the caller supplied and does no path confinement of its own.**
  That is deliberate (the frontend owns trust), but it means the tool's safety is exactly the
  safety of the list it was constructed with — there is no second check inside `execute`.
- **`disable-model-invocation` filtering has no test at the CLI seam.** The two tests are in
  `epsilon-core`'s `catalog`; the CLI's own `.filter(|s| !s.disable_model_invocation)` before
  building `LoadableSkill`s is covered by reading, and by the type having no flag to leak.
- **The user-facing `/skills` list is unchanged and still shows disabled skills** — which is
  correct, because it is the user's view, but it is worth stating so a later reader does not
  "fix" it into agreement with the catalog.
- **Deliberately not claimed here** (named in the first commit as still open from the same
  audit): `.gitignore` is still promised by the `grep` and `find` descriptions without being
  implemented — closed in M99.
- **Windows-only verification.** Nothing here is platform-specific; the Linux CI job has not run
  on these bytes.

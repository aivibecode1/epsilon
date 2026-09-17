# M82 — Plan-mode shell refusal and rule layering (F10)

Plan mode previously advertised a read-only boundary it could not enforce. Two defects
shipped together here because both are the same user-facing promise: `--plan` is research-only.

## Change

- `plan_ruleset()` now denies `bash` and `shell` outright. The old rules matched
  `git commit*`/`git push*`, but core preflight authorizes shell calls by command HEAD only
  (`exec.rs` extracts the first whitespace word), so those subcommand patterns could never
  match a real subject. Classifying arbitrary command strings as "read-only" is not portable;
  refusal is the honest boundary.
- `build_runtime` (epsilon-cli) composes project `[[rules]]` UNDER the plan layer instead of
  replacing them. Monotonic deny keeps plan denies winning; project denies survive plan mode;
  non-plan branches are unchanged.

## Tests (red first, both slices)

- Engine slice: `plan_mode_denies_bash_even_when_project_allows_it` failed pre-fix
  (Allow vs Deny) in loop_with_rules; new `tests/plan_shell.rs` proves through a real
  Agent that bash/shell calls NEVER reach tool execution despite a project allow and
  AutoApprove, across both tool names and five commands including empty.
- CLI slice: `runtime_tests::plan_mode_preserves_project_read_deny` loads a real project
  config through production `build_runtime`; project read-deny still refuses in plan mode,
  ordinary reads stay allowed, and a project write-allow cannot punch through plan deny.
  Red evidence shows the private read succeeding before the fix.

## Verification

Five-gate battery `gate.sh f10-composed`: GATE PASS (fmt=0, clippy=0, test=0, demo=0,
real-check=0), including the previously flaky wire_fixtures link. Evidence preserved in
`audit/evidence/` (f10-*.log, gate log, narrow delta patch). The earlier interrupted round
is recorded in `audit/evidence/f10-interruption.txt` without being rewritten as success.

Post-additive-authorization rerun `gate.sh f10-final2` (documented Avira workaround
target dir): GATE PASS, workspace suite **794 passed / 0 failed / 1 ignored** across 55
summaries — the first full battery over the final authorization code. Independent
read-only review of the authorization delta: **approve, no blockers**
(`audit/evidence/f10-review-approve.json`). Full record:
`audit/05-verification-log.md` § "F10 final authorization round".

## Scope limits

- Residual races and symlink identity are F09 territory, not claimed here.
- Plan-mode subject extraction for other tools is unchanged.
- Out-of-workspace subjects keep raw spelling (consistent, no bypass; prompt-dedup
  limitation noted by review).


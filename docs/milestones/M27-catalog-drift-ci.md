# M27 - Weekly catalog-drift CI job

Date: 2026-08-26. Closes the last open item on the README's future row. The
future table now reads "nothing scheduled"; new items enter it when someone
names one.

## What

A new workflow, `.github/workflows/catalog-drift.yml`:

- Runs every Monday at 04:23 UTC (and on demand via workflow_dispatch).
- Runs `python scripts/regen_catalog.py`: downloads live
  https://models.dev/api.json and rewrites
  `crates/epsilon-llm/src/catalog/catalog_data.rs`.
- Applies `cargo fmt -p epsilon-llm` (the generator emits pre-fmt source).
- If nothing changed: exits quietly. No commit, no PR, no noise.
- If something changed: force-pushes ONE standing branch,
  `catalog-drift/latest`, with a single regenerated-file commit, then opens
  or re-titles the pull request against main.

## Why a PR instead of pushing main

The generated catalog is not always a pure data refresh: catalog tests pin
specific rows (`catalog_is_current_generation`), so a registry shuffle can
move a pin and fail CI. Routing the change through a PR means the normal
gates (Windows full suite, Linux confinement, MSRV, fixtures) run on the
refresh exactly like any other change, and the failing test names the row
that needs a human decision. Pushing straight to main would trade that
safety for convenience on the one file where breakage is plausible.

Also matches repo law: never push main directly; PRs are where review lives.

## Cost and scope discipline (ponytail)

- No cargo build in the job: one Python download, one rustfmt, one git push.
  Minutes stay tiny even on a private-repo billing plan.
- One standing branch, force-pushed each time — no weekly branch/PR litter.
  A closed PR reopens naturally when the branch updates; `gh pr edit`
  refreshes title/body when it is still open.
- No new test logic anywhere. The drift detection is `git diff --quiet`.

## Failure modes considered

| Failure | Behavior |
|---|---|
| models.dev unreachable | generator's urllib raises; job fails red (visible, honest) |
| Generator crashes on schema change | same: red run names the step |
| Registry rows moved that a test pins | PR goes red under normal CI; fix the pin, merge |
| No drift | silent success (no commit exists to make) |
| Branch protection blocks bot push | red run; adjust protection rules then |

## Verification

- YAML parsed clean (`yaml.safe_load`), LF line endings.
- Local dry-run of the pipeline steps on this machine:
  regeneration against a cached models.dev snapshot produced a byte-identical
  catalog_data.rs (no drift today, as expected after M20's regen) — so the
  first scheduled Monday run should also be quiet unless the registry moves
  this week.
- Real end-to-end proof lands with the first scheduled run or a manual
  workflow_dispatch fire; the job is 4 small steps, all individually logged.

## Docs

README status row (M27) + Status paragraph naming the weekly job. CHANGELOG
entry. No ADR amendment needed: no product behavior changes, this only
automates an existing documented manual procedure (M20's regen loop).

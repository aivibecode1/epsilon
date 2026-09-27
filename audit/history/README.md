# Earlier assessments

These are the audits this repository accumulated between 2026-09-16 and
2026-09-27. They are kept because the reasoning in them is worth reading and
because deleting a record of what someone believed is its own kind of dishonesty.

**They are not current.** The live assessment is in the parent directory.

## What to know before reading them

Two of these documents make claims the current audit disproves. They are
marked where they are wrong rather than quietly left to mislead:

- **`11-current-assessment.md`** and **`history/…/tui-redesign.md`** record that the
  typing-freeze report "was two real defects, both already fixed and both measured".
  The current audit [../04-tui.md](../04-tui.md) reproduces the report, shows the
  real cause, and shows that the measurement which established the "fixed" claim
  never typed while a run was in flight — the only state the report describes.
- **`02-defect-ledger.md`** is the origin of several rows that are still true and
  some that are not. Its status column was the closest thing this repository had
  to a work queue, and it is why the current audit re-derived every row from the
  code instead of inheriting it.

## The rest

| Document | Date | Note |
|---|---|---|
| `01-usability-review.md` | 2026-09-16 | The first pass. Superseded by the revalidation, which corrected it. |
| `02-defect-ledger.md` | 2026-09-16 | The defect rows. Several were re-confirmed from code and carry forward. |
| `03-port-plan.md` | 2026-09-16 | pi parity matrix. Superseded by the current [../02-port-fidelity.md](../02-port-fidelity.md). |
| `04-roadmap.md` | 2026-09-16 | The old sequencing. Historical. |
| `05-verification-log.md` | 2026-09-16 | A **baseline** record, ending at M86–M88. Not a current log; the tail is a hundred milestones stale and the file says so in its own header. |
| `06-reference-comparison.md` | 2026-09-16 | Superseded by the current [../06-references.md](../06-references.md). |
| `07-inconsistencies.md` | 2026-09-16 | Internal contradictions found by the revalidation. |
| `08-restructure-record.md` | 2026-09-16 | A past restructure. |
| `09-revalidation.md` | 2026-09-19 | Source-level revalidation of the rows above. Its corrected severities are the ones worth remembering. |
| `10-tui-enhancement.md` | earlier | An earlier TUI round; the presentation layer has since been rebuilt. |
| `11-current-assessment.md` | 2026-09-19 | The assessment this audit replaces. |
| `12-fresh-reference-audit.md` | 2026-09-27 | A second reference pass, written the day before this audit began. |
| `00-revalidation-scope.md` | 2026-09-19 | Scope and corrections for the revalidation. |
| `references/` | 2026-09-21 | Reference-harness notes kept alive so a `git clean` cannot destroy a line the documents pointed at. |

`evidence/` was left in place at `audit/evidence/` because the gate logs and probe
results in it are still cited by the current audit.

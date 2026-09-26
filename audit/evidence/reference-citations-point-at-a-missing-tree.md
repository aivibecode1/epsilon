# The port-fidelity evidence base cites a reference tree that is not present

Captured, not composed. Run `python scripts/check-reference-citations.py` to reproduce.

## The finding

`references/` contains four checkouts. **One of them is not `pi-0.84.2`:**

```
deepseek-harness-dsh-v0.1.6-alpha.2
hermes-agent-2026.9.14
opencode-1.18.31
pi-0.85.1
```

The reference harnesses were updated, as the owner said. But `audit/03-port-plan.md` still
declares on line 22:

> For compactness, pi paths below start with `references/pi-0.84.2/`

and line 4:

> **Reference:** bundled `references/pi-0.84.2/`; current Rust working tree over `e200815`, 2026-09-16.

**`references/pi-0.84.2/` does not exist.** So does the second half of line 4: the Rust tree is
now over `9c61af7`, not `e200815`.

Counts: **97 occurrences of `pi-0.84.2`** across 12 files in `audit/`, `docs/` and `crates/`,
versus **28** of `pi-0.85.1`. Of the 97, **54 carry a file path and line range**; the rest are
bare version mentions in prose.

## What that does and does not mean

**It does not mean any finding is wrong.** It means the *evidence* for the port-fidelity claims
is currently unverifiable, which is a different and more serious problem: an unverifiable
citation is indistinguishable from a fabricated one.

The checker therefore reports two objective facts and stops:

- does the cited file exist under the checkout we actually have, and
- is the cited line range inside that file.

It deliberately does **not** try to confirm that the cited lines say what the document claims,
because that judgement is not mechanical — and a script that guessed it would be exactly the
"plausible-looking fabricated output" this repo forbids.

After the two fixes below, all 54 path-bearing citations are checkable. **Checkable is not
verified.** A version bump shifts line numbers, and a citation can sit comfortably in bounds
while pointing at unrelated code. Every one of the 54 still needs a human or an agent to read
the cited lines and confirm the claim.

## The two that were provably broken

Both found by the checker, both in `audit/03-port-plan.md`:

**1. `packages/coding-agent/src/core/tools/read.ts:209–268` — past end of file.**
In 0.85.1 that file is **198 lines**. The cited range does not exist. The image pipeline did
not merely move lines, it was **extracted**: `read.ts:41–57` now holds the `detectImageMimeType`
and `autoResizeImages` options, and the work lives in
`packages/coding-agent/src/utils/image-process.ts:87` (`resizeImage`) and
`packages/coding-agent/src/utils/image-resize.ts`. Citation corrected to those.

This is the shape of the risk. A reader following the old citation finds a file that exists,
in a plausible package, doing image-adjacent work — and no reason to suspect the numbers are
from a different version.

**2. `packages/agent/package.js` — a build artifact.** Cited as evidence of what the
`packages/agent` directory *is*. The right target is the manifest,
`packages/pi-0.85.1/packages/agent/package.json`.

**Correction to the record:** this second one was initially reported as a *bad citation*, and it
was not. It was a bug in the checker I wrote: the extension alternation tried `js` before
`json`, so `package.json` matched as `package.js` and the `:2–4` range was dropped. The regex
is fixed (`(?:tsx|ts|json|js|md)` plus a `(?![A-Za-z])` guard) and the count moved 55 → 54 with
zero failures. **A tool reporting on the repo's own evidence is itself evidence, and it has to
be checked before its output is believed.**

## What is left

- **Re-verify all 54 path-bearing citations** against `pi-0.85.1` by reading the cited lines.
  Until then the port plan's "source-verified" claim on line 20 is **not true of this file** and
  should not be repeated in any summary.
- The other 43 are bare version mentions in prose; they need a version string updated, not a
  line check.
- `audit/03-port-plan.md:4` also pins the Rust tree to `e200815`; every such pin in `audit/`
  is now stale in the same way.
- Six other reference harnesses (DSH, Hermes, opencode) are cited with versions that *do*
  resolve. The breakage is specific to pi, because pi is the one that was updated.

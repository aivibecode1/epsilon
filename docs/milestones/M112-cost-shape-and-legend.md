# M112 — the cost model gets a shape, its dead twin leaves, and the header gets a legend

Three small changes that belong together because they are all about the same thing: the
numbers in the header have to be *readable*, and the code behind them has to say what it
means.

## What

**The cost model returns components, not just a total.** M111 put the four-bucket
arithmetic in `ui/cost.rs` with a total and a floor flag computed by two passes over the
same prices. `CostBreakdown` now carries the four components, with the unscaled products
kept private so [`total`](../../crates/epsilon-tui/src/ui/cost.rs) can sum them and divide
**once** — the property the previous shape had, and the one a first attempt at this
refactor quietly lost (a test caught it: two half-micro-USD components divided separately
total to zero). The floor flag is derived in the same pass that takes the products, so the
number and its caveat cannot disagree, and `App::metrics` builds `TokenPrices` once
instead of twice.

**`CostMicro` left `epsilon-contracts`.** A second per-rate cost type, with an `Add` impl,
that nothing in the workspace constructed or read — dead code of the kind M107 found in
the TUI, this time in the crate whose stated rule is *types and traits only*, where an
`Add` impl is behaviour it is not supposed to have. Two types for one concept is how a
reader ends up implementing against the wrong one; the live type is the one beside the
arithmetic that uses it.

**`/help` decodes the header.** A user saw

```
idle                    ?/164k · ↑0 ↓0 · cache — · $0 · DeepSeek V3.1
```

and nothing anywhere explained the `?`, the `—`, the arrows, or what the cost is a
function of. Three lines now say what each figure is, including the two placeholders that
mean *unknown* rather than *zero* and the trailing `+` that marks a lower bound.
Transparency that needs a decoder ring is only transparent to the person who wrote it.

## Key decisions

- **The components are `scaled` and private.** The obvious shape — four public `u64`
  fields in micro-USD — makes `total()` a sum of already-truncated parts, which is exactly
  the four-truncations bug M111 fixed. Keeping the products unscaled and reading them
  through accessors costs a line of documentation and buys the correct total.
- **`is_floor` is a field, not a recomputation.** It is decided where the products are
  taken, so a future change to the pricing rules cannot update one and forget the other.
  M111 computed it from a second expression over the same inputs; that is the shape that
  drifts.
- **A free model is not a floor.** With M111's rule, a `:free` registry row or a local
  ollama model — every rate zero — would print `$0+` for a session that genuinely cost
  nothing. The rule now requires that the registry prices the model *somewhere* before a
  zero bucket counts as a gap. The test `a_model_that_costs_nothing_is_not_a_floor` is the
  one that would fail under the looser rule.
- **Delete the duplicate rather than adopt it.** `CostMicro` could have been used as the
  return type of `breakdown`, which would have given a dead type a home and dragged a
  behaviour-carrying contracts type into a presentation module. Deleting it is smaller,
  and it removes an invitation to implement against the wrong cost type.

## Gotchas

- **A refactor can silently undo a property the tests name.** The divide-once rule was
  covered by a test written two rounds earlier, and restructuring the function for
  components broke it immediately. That is the test doing its job; it is worth noting that
  the failure looked like an arithmetic surprise rather than a regression.
- **The dead type was invisible for the same reason as M107's dead footer**: `pub` in a
  library crate is exempt from `dead_code`, so `clippy -D warnings` cannot see that nothing
  calls it. Two instances now found by reading rather than by tooling.
- **The legend names the caveats, not just the numbers.** The `+` and the `—` exist because
  the harness refuses to fabricate values; leaving them unexplained would have undone that
  refusal at the last step.

## Verification

Battery `m112a` (`target/gate-m112a.log`, copied to
[`../../audit/evidence/gate-m112a.log`](../../audit/evidence/gate-m112a.log)): **all five
gates exit 0, 1,032 passed / 0 failed / 1 ignored across 59 test binaries** — one more than
M111's 1,031, which is `a_model_that_costs_nothing_is_not_a_floor`. `epsilon-contracts`
reports its 25 tests unchanged by the `CostMicro` deletion, which is the evidence that
nothing referenced it; `epsilon-tui` 227.

Reinstall: **TUI behaviour changed** (the `/help` body and the cost model), so the global
`epsilon.exe` is rebuilt from this round's tree.

## Limits

- **No test asserts the `/help` lines.** They are strings in a match arm; a typo or a
  deleted line would be caught by reading, not by the suite. Testing them properly means
  asserting on a notice list, which is worth doing when `/help` next changes for another
  reason.
- **The breakdown has no consumer for its components yet.** `total()` and `is_floor()` are
  what the header uses; the four accessors exist for a `/cost` view that does not exist.
  They are tested, so they are not speculative in the "unreachable" sense, but they are
  unused in the product.
- **The zero-price ambiguity remains.** M111's Limits named it and this slice only narrows
  its blast radius: a registry row that publishes *no* price and one that publishes a price
  of *zero* both arrive as `0`. The rule "the model is priced somewhere" separates the
  common cases (a free row, a local model, the offline provider) from a paid model with an
  undescribed bucket, which is the case the marker is for. Distinguishing the remaining
  overlap needs `Option<u64>` in the schema.

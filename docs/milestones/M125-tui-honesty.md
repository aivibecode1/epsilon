# M125 — an honest footer, a true context number, and an approval box that fits

The TUI audit's items 3, 4 and 5 — the three that remain once the decoration (M123) is actually
drawn.

## What

**3. The footer now describes the harness that is running.** Hints were literals, and
`Hints::idle()` advertised `? help` — but `?` is not a keybinding; it only works as text parsed
into `Command::Help`, so the hint named a key that does nothing until the user presses it *and*
Enter. Meanwhile the keys a real session needs most — PageUp/PageDown, Up/Down history, Ctrl+O
external editor, Ctrl+X expand — went unmentioned. Hints are now resolved from the **live binding
table** (`bindings.push("app.transcript.pageUp", "scroll back")`), so a rebound key changes its own
hint and a dead hint is not representable. Scrolling goes early in the line, because while the
model is talking that is what a reader reaches for. The false `enlarge the terminal (need 60×12)`
message is gone: it contradicted the real floor of 44×10 in `ui/layout.rs`, and now formats from
those constants so the two cannot drift apart again.

**4. The header stopped printing a number that flatters.** `context_used` was
`latest_prompt_tokens` — the *last call's* prompt size — rendered as `42%/200k`, which reads as
window occupancy. A true number and the wrong one: the two agree on turn 1 and diverge after, and
the divergence is always flattering. It is now the session context — the tokens the **next**
request will carry — which is the same number the compaction trigger reads, so the header and the
thing that decides to compact can no longer disagree. The identical `None` arms in `context_gauge`
went with it.

**5. The approval dialog fits its question.** 11 rows → ~6 (the skill asks 5–7 for a yes/no; a tall
box for a one-line question reads as more consequential than it is), the height computed once from
named row constants so it cannot be given a size it does not have, and it now actually uses the
danger accent its own doc comment had claimed all along — `Panel` always took `border_focus()`, so
the comment was false about the code.

## Key decisions

- **Feed the real number rather than relabel the wrong one** (item 4). The audit offered both and
  asked for the choice to be reasoned: labelling it `prompt 42%/200k` would have been honest and
  still useless for the question a user actually asks ("how much room is left?"). The chosen number
  is the one the compaction trigger already trusts, which also removes a chance for the header and
  the compactor to disagree.
- **A hint is derived, not written.** The reason `? help` could sit in the footer for so long is
  that the footer did not know what the keymap said. Deriving it from the binding table makes that
  class of lie unrepresentable rather than merely fixed.
- **Cells from the real draw path** remain the test standard for anything visual (set in M123).

## Verification

Battery `m125a`: **all five gates exit 0, 1,116 passed / 0 failed / 1 ignored across 61 binaries**
— eighteen more than M124's 1,098, which is this slice's new tests. `epsilon-tui` alone: 252
passed (was 234). fmt and clippy clean.

## Limits

- **The context figure is an estimate before the first provider reply.** The provider-anchored path
  is exact; the pre-first-reply path is `chars/4` and can be off by the factor `epsilon-core`'s own
  comment warns about (a resumed session especially). It is narrower than the number it replaced,
  never wider, and never rounded up — **but the row does not say "estimate"**, and it should.
- **The new scroll hints cost two hints at 80 columns.** `ctrl+j newline` and `ctrl+p palette` no
  longer fit; the near end yields first. The trade is deliberate and asserted
  (`what_survives_a_narrow_row_is_the_hint_that_cannot_be_guessed`), not accidental.
- **Up/Down history recall is still unadvertised.** The audit lists it, but it has no binding id —
  `KeyCode::Up` is handled directly in the composer. PageUp/PageDown got ids precisely so their hint
  cannot drift from their handler; history should get the same treatment.
- **The author of this slice reported after the commit**, so the first commit's message says it never
  reported. It did: the report arrived with **five negative controls** (each restored byte-identically
  and touched), the item-4 decision and its reasoning, and three disclosures now recorded above. Two
  refinements it made after the first commit landed in `fe1888c`, and `m125a` ran on the code as it
  stood before those two files changed — the delta is a dead-accessor deletion and a test message, no
  behaviour, but the ordering is stated rather than glossed.
- **The independent read-only review the milestone workflow asks for is still owed.**
- **The audit's larger items remain**: tool cards with a real coloured diff (today a diff is
  uncoloured markdown prose behind a global toggle), a light theme (`Color::White` is still body
  text), a frozen-view signal and jump-to-live, `NO_COLOR` glyph separation for the remaining roles,
  and OSC 52 for `/copy` over SSH.

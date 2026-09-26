# M151 — the welcome screen stops repeating the footer, and the cache row gets a denominator

## Why this milestone exists

The owner saw a real screen, taken by `scripts/console-probe.py` against this build in a
100×30 legacy conhost, and said the design was far below the references. That criticism is
correct, and this milestone is the part of it that could be fixed honestly in one sitting.
It is deliberately **not** presented as closing the gap.

## What was actually wrong

The probe frame, verbatim, before this change:

```text
idle                    ?/164k · ↑0 ↓0 · cache — · DeepSeek V3.1

E P S I L O N
a coding agent in your terminal

  type      describe a task and press enter
  /         commands — try /help, /models, /sessions
  @         reference a file by name
  ctrl-p    command palette

  this session runs against the model shown at the top right
…
• ask anything
────────────────────────────────────────────────────────────
insert · enter send · pageup scroll back · pagedown scroll forward · …\My Agent Space\Epsilon · main
```

Two defects, both invisible to every test because both are questions of *taste* that the code
was written to assert:

1. **The keybindings were printed twice.** The footer renders the *live* key set for the current
   state; the welcome block rendered a second, static copy three lines above it. The upper copy
   is wrong the moment the user presses a key — `enter send` becomes `esc stop` — so the screen
   shows two contradictory answers to the same question, and the stale one is the larger.
2. **The cache row had a rate with no denominator.** `cache 75%` is 75% of *what*? A 200-token
   prompt and a 2M-token prompt produce the same string and mean opposite things. And when the
   rate was unknown the row read `cache —`, which is **not the same as no cache**: a session that
   had already cached 400k tokens looked like it had cached nothing.

## The change

The welcome screen now takes `&Metrics` and a session label, and spends its space on what a user
cannot read anywhere else — what this session actually is:

```text
idle                    ?/164k · ↑0 ↓0 · R0 W0 CH — · DeepSeek V3.1

E P S I L O N
a coding agent in your terminal

  model    DeepSeek V3.1
  context  ?/164k

  ready — describe a task and press enter
```

and the cache fragment is `R<read> W<write> CH<rate>` — the same breakdown pi's footer prints
(`footer.ts:130-145`). The rate is withheld when it would be meaningless, but the volume is
always shown, because the volume is what the rate was a fraction *of*.

## Negative controls

- `the_welcome_screen_does_not_repeat_the_footers_keybindings` — asserts `ctrl-p`,
  `command palette` and `pageup scroll` are **absent**. Reverting the design change fails it.
- `a_session_label_appears_only_when_there_is_one` — an empty label drops the row rather than
  printing a bare `session` field.
- `cache_rate_is_about_the_latest_prompt_not_the_session` kept its meaning: only the expected
  *string* changed, from `cache 75%` to `R18k W2.0k CH 75%`. The rate is still computed from
  the latest prompt, and `a_tiny_prompt_does_not_claim_a_cache_hit` still proves it.

One of the four failures during this round was mine and is worth recording: a blanket
search-and-replace of `"cache —"` gave the wrong expected string to a fixture that *did* have
18k of session cache. The failure was the correct outcome — the assertion had to distinguish
"no cache" from "cache but no meaningful rate", which is the whole point of the change.

## The gap this does not close

Measured, not guessed:

| | Epsilon | pi |
|---|---|---|
| UI code | 25.5k lines, bespoke frame renderer | 36k-line `packages/tui` |
| Structure | 12 modules, hand-built `Rect` math | component framework: `box`, `stack`, `scroll-view`, `select-list`, `markdown`, `editor`, `fuzzy`, `autocomplete`, `latex`, `mermaid`, `image` |
| Interactive components | — | 44 |
| Themes | one, plus an ASCII glyph set | registered theme set, theme files, auto-detection of terminal background, a theme selector |

Ours is a competent hand-rolled renderer; theirs is a framework with a design system on top.
Closing that is a multi-milestone project, not a slice, and pretending otherwise by adding
hardcoded strings would make the codebase worse while making this note longer. What this
milestone does is remove a real defect and add one real capability, both measured in a real
console window.

**What nothing exercises:** the session label is wired from `App::session_name`, which is
`None` for every session this host has run, so the `session` row has never rendered in a probe —
only in a test. The theme gap above is unmeasured by anything; it is a code-reading claim about
what each side contains, and it is the next thing to plan properly rather than the next thing to
patch.

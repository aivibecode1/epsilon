# M16a — Rendering efficiency + footer parity

Date: 2026-08-24 · Status: shipped. Plan first (`M16a-plan.md`), bench before/after,
then the render-cache rewrite. The user's complaint to fix: the TUI must get *faster*
with longer sessions, not slower.

## What was built

### 1. `--tui-bench <n>` (the honesty meter)
Hidden CLI mode that builds `n` synthetic messages (markdown-rich assistants +
tool results) through the REAL event reducer, then times 300 production draws on a
ratatui `TestBackend` (110×40). One persistent `App` across frames — the same shape
as the live loop (an earlier draft rebuilt the App per frame and measured garbage;
caught immediately by comparing against the BEFORE baseline).

### 2. Render cache + virtualized transcript
- Every `TranscriptLine` now carries a unique monotonic `id` (process-wide atomic;
  ids survive `/clear` without reuse). This is the cache key.
- New `render_cache.rs`: id → wrapped screen rows, keyed per width + expand flag.
  Markdown parsing (pulldown-cmark) and wrapping happen ONCE per line, at commit.
- Reconcile is incremental: the common id-prefix with the previous frame is reused,
  so steady-state reconcile is O(1); only genuinely new lines cost work. `/clear`
  and width changes invalidate structurally (rare).
- `draw_transcript` materializes ONLY the visible window: prefix-sum offsets +
  binary search for the window start, then clone just those rows. Streaming text
  still rides under the cached rows live.
- Own wrap (`wrap.rs`, unicode-width): greedy word-wrap preserving span styles,
  hard-splitting oversized words, dropping trailing whitespace at wrap points.
  Replaces `Paragraph::wrap` because virtualization needs row counts decided
  ahead of the draw.

### 3. Picker row caching
`build_rows` previously ran on every Up/Down/mouse/draw. Rows are now cached per
(filter, recents, live) tuple; typing/backspace/live-fetch/recents-change
invalidate explicitly; navigation is pure index math over the cached vec.

### 4. pi footer parity (`footer.rs`, pure + tested)
Status bar now answers pi's questions in one line:
```
<spinner> status · elapsed     ~\path (branch) · ↑in ↓out · R/W · CH% · $cost · 12%/200k · Model Label
```
- cwd ~-shortened via `PathBuf::join` (per-OS separators stay consistent);
  git branch read from `.git/HEAD` directly with a 2s TTL — no git dependency.
- Compact tokens exactly pi's `formatTokens` rules (k/M bands, rounding).
- Context % = latest prompt (input+cache_read+cache_write) ÷ catalog context_window,
  yellow >70%, red >90%, **`?` right after compaction** until fresh usage arrives.
- CH% cache-hit rate surfaced — pi's headline metric (deep-dive transcript:
  pi wins on price/speed *because* of its cache discipline; showing it keeps us
  honest about our own byte-stable prompt tiers).
- Cost stays integer micro-USD internally (ADR-010); unknown models omit cost
  honestly; fresh session renders no fake zeros.

## Bench numbers (debug build, 110×40, 300 timed draws)

| Messages | Screen rows | BEFORE µs/frame | AFTER µs/frame | Speedup | AFTER share of 120ms tick |
|---|---|---|---|---|---|
| 200 | 469 | 48,168 | ~1,600 | ~30× | 1.3% |
| 400 | 936 | 99,029 | ~2,156 | ~46× | 1.8% |
| 1000 | 2336 | (extrapolated ~247,000) | ~3,585 | ~69× | 3.0% |

The shape matters more than the ratios: BEFORE grew linearly (~0.25ms/message),
AFTER is near-flat — scrolling a 1000-message session costs less than 3% of the
frame budget. Acceptance criterion "no perceptible lag on 200 messages" passes with
two orders of magnitude of headroom.

## Test-caught bugs (all mine, all fixed)

- **Duplicate cache ids**: my first id design used a per-state counter but the line
  constructors initialized it locally — every ad-hoc line got id 1 and the cache
  collapsed distinct lines into one entry. Fixed with a process-wide atomic source;
  uniqueness holds regardless of who builds the line.
- **Offset table off-by-one after invalidation**: clearing offsets without re-seeding
  `offsets[0] = 0` shifted every window by one row (tests showed "line 19" where
  "line 20" belonged). The base entry is now part of the invalidation path itself.
- **Borrow conflict**: `picker_rows()` (needs `&mut self` to fill the cache) vs the
  cloned fetcher handle in `fetch_live_for_selected`. Cloned the handle first.
- **Footer fit underflow**: `(width - right_len)` panicked when stats exceeded a
  narrow terminal; now saturating arithmetic throughout.
- Test-expectation errors I then had to own: CJK width math (2 cols each), token
  band boundaries (6_000 → "6.0k", inside the <10k one-decimal band), Windows path
  separator expectations built from literals instead of PathBuf.

## Decisions

- Virtualization over dirty-tracking: wrapping is deterministic per (id, width),
  so a full-window cache gives O(visible) draws AND correct heights for scroll math;
  dirty-prefix rebuilding would still pay O(transcript) per frame for offsets.
- `unicode-width =0.2.2` exact-pinned: already in-tree via ratatui-core (>=0.2.0);
  zero new supply chain, satisfies ADR-016.
- No ADR amendment needed: this is an internal rendering implementation change inside
  epsilon-tui; crate boundaries, event bus, and approval flow untouched.

## Verification

```
cargo fmt --all -- --check              # clean
cargo clippy --workspace --all-targets  # zero warnings
cargo test --workspace                  # all green (see totals below)
cargo run -q -p epsilon-cli -- --demo   # completes, reason: Completed
cargo check -p epsilon-cli --features real
cargo run -q -p epsilon-cli -- --tui-bench 400   # report above
```

New tests: bench fixture/report shape (2), wrap word-boundaries/hard-split/CJK/
style-preservation (7), render cache once-only/windowing/stitching/clear/width/
expand/prefixes (7), footer format/cwd/branch/CH%/context-thresholds/stats-shape (6).

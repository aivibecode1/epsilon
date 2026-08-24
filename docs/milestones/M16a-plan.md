# M16a plan — rendering efficiency + footer parity

## Findings (read before writing)

- `transcript_rows()` (app.rs:780) re-runs pulldown-cmark for EVERY assistant row and
  clones every line's strings on EVERY draw (~8 draws/sec at TICK_MS=120); then
  `Paragraph::wrap` re-wraps everything. Cost grows linearly with transcript length.
- Picker calls `build_rows` on every Up/Down/mouse/draw — refilter + realloc each time.
- Footer lacks pi parity: no cwd/branch, no compact tokens, no context-%, no cache-hit
  rate. Catalog carries `context_window`; contracts `Usage` carries cache_read/write;
  events carry CompactionStarted/Finished — all inputs already exist.
- Deep-dive insight: pi's headline advantage IS its cache-hit rate → surface CH% in UI.
- `Paragraph::scroll` skips SCREEN rows after wrapping, so honest virtualization needs
  our own deterministic wrap (also fixes mid-word wraps on long tool lines).
- ratatui TestBackend is public → real timed draws headless. unicode-width 0.2.2 already
  in-tree via ratatui-core (needs `>=0.2.0`) → exact-pin, zero new supply chain.

## Edit list

1. `state.rs`: monotonic `id` on TranscriptLine (assign at push/clear); track session
   totals incl. cache fields + latest prompt tokens (in+cache_read+cache_write) +
   `compacted: bool` (set on Compaction* events, cleared by next assistant usage).
2. NEW `wrap.rs`: greedy word-wrap `Vec<Span>` → rows at width W (unicode-width),
   paste-marker-safe, unit-tested.
3. NEW `render_cache.rs`: width-keyed {id → wrapped rows} + prefix row-count sums;
   API: `rows_for(window)` materializing ONLY visible rows. Miss = render once.
4. `app.rs` draw path: transcript = cache lookup + integer binary search + blit of the
   visible slice (no md parse, no clones, O(visible)); streaming tail appended fresh;
   `x` toggle + width change invalidate cache. Picker rows cached per filter string,
   invalidated on recents/live/model change.
5. NEW `footer.rs` (pure, tested): `format_tokens` (pi k/M rules), `format_cwd`
   (~-shorten), `git_branch` (read `.git/HEAD`, 2s TTL, no deps), `context_str`
   (`12%/200k` or `?/200k` right after compaction), `ch_rate`. Status bar becomes
   pi-shaped: left status/spinner, right `~path (branch) · ↑in ↓out · CH% · ctx · $cost · model`.
6. Bench FIRST: `--tui-bench <n>` hidden mode (CLI parse → `tui::bench::run`) builds n
   synthetic messages, times 300 draws on TestBackend(110, 40), prints µs/draw + rows.
   Capture BEFORE numbers with current pipeline, THEN land 1–4, capture AFTER.
7. Gates: fmt/clippy-zero/tests/demo/real-check green; key-path tests updated for
   picker-cache invalidation; new unit tests per pure module.

## Non-goals

Editor v2 + palette (M16b/c). No gitoxide/arboard/fuzzy-matcher deps. No redesign.

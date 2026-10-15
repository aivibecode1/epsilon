# Provenance

Filed 2026-10-15 as part of wave 19 (`19-settings-changes-2026-10-15.md`, which holds the verdicts). This is the audit child's report as written to scratch during delegation `deleg_ade17047`, unedited except for this header. Repo-side claims in it are leads until `19-...` says re-verified.

---

# Epsilon command-registry honesty audit — 2026-10-15

Scope: `crates/epsilon-contracts/src/command.rs` (SLASH_ITEMS, COMMAND_HEADS, Command, parse_command)
→ `crates/epsilon-tui/src/app/commands.rs` (run_command + 6 executors) → picker autocomplete/palette, keybind tables.
Read-only; nothing compiled. All verdicts below are **read** unless marked (inferred).

## Q1 — End-to-end trace (advertised → parsed → executed → feedback)

| Command | Parse | Executor arm | Feedback / errors | Verdict |
|---|---|---|---|---|
| /models, /model | command.rs:534,544 | commands.rs:749-781 (provider) | picker opens; ambiguous id re-opens picker with notice; never saves typo | LIVE |
| /think | command.rs:545 | commands.rs:782-812 | bare opens level picker; bad level names valid levels | LIVE |
| /compact | command.rs:550 | commands.rs:1852-1893 | already-running / queued / failed-compaction notices | LIVE |
| /theme | command.rs:551 | commands.rs:1823-1851 | toggles or sets; unknown name refused by name (**but message stale — see F2**) | PARTIAL |
| /timeline, /branch, /tree | command.rs:556-557 | view 2488-2490 / session-navigate 1387-1389 | picker opens; empty-store/store-errors named | LIVE |
| /goal | command.rs:558 | commands.rs:997-1000 → run_goal_command 1026-1079 | every path pushes transcript notice; invalid verbs named | LIVE |
| /fork, /clone, /name | command.rs:563,568,591 | run_branch_command 1447-1588 | picker / store-error / usage / persist-failure notices | LIVE |
| /new, /resume, /sessions, /session | command.rs:569-573 | navigate 1315-1353 / restore 1594-1675 | busy guards, missing-file, no-store notices | LIVE |
| /trust, /reload | command.rs:574,579 | transfer 2620-2697 / restore 1676-1778 | hash-state sentences; discovery notices forwarded verbatim | LIVE |
| /export, /import | command.rs:580,585 | transfer 2698-2784 | success counts; store-missing/file-missing/busy errors; import confirms | LIVE |
| /open | command.rs:586 | workspace 1906-1912 | **honest refusal by name**, points at /resume. SLASH_ITEMS description (command.rs:180-181) discloses terminal refuses. Not a no-op — a documented refusal | LIVE |
| /restore | command.rs:587 | workspace 1916-1918 → run_restore_file 2122-2160 | preview then `yes`; busy guard; store refusals passed through | LIVE |
| /changes | command.rs:588 | workspace 1919-1921 → open_changes 1957-1981 | bad scope refused by name (1960-1963); session/all scopes; see F3 | LIVE |
| /share, /copy | command.rs:589-590 | transfer 2785-2874 | gist-or-deferred sentences; clipboard transport + empty notice | LIVE |
| /changelog, /tasks, /hotkeys, /settings, /catalog | command.rs:596-599 etc. | view 2349-2559 | missing-file fallback, empty-state sentences, effective bindings listed | LIVE |
| /skills, /templates, /skill, /search | command.rs:609-613 | view 2365-2426 / session 956-996 / workspace 1809-1821 | empty-state guidance; **/skill error paths quit — see F1** | PARTIAL (skill only) |
| /login,/connect,/auth,/logout | command.rs:614-637 | provider 832-928 | picker / masked prompt / save+post-login / usage+remove results | LIVE |
| /clear, /expand | command.rs:607-608 | workspace 1787-1808 | totals-kept / expanded-collapsed read back from state | LIVE |
| /help, /quit,/q,/exit | command.rs:606,638 | view 2299-2348 / workspace 1822 | prints SLASH_ITEMS + guide; Quit returns true | LIVE |
| /scoped-models,/scoped_models | command.rs:600 | provider 813-831 | checklist opens; trust-gate refusal named | LIVE |
| /preset | command.rs:601 | session 955 → run_preset_command 1411-1445 | lists four; unknown named; privilege raise confirms | LIVE |
| /editor | command.rs:572 | workspace 1913-1915 | request_external_editor (inferred: OS-handoff, no transcript line — standard) | LIVE |
| /unknown | command.rs:639 | workspace 1894-1901 | names word + suggest_slash hint | LIVE |

**No registered-but-unreachable and no advertised-but-no-op command found.** The catch-all `other` arm (commands.rs:1940-1947) pushes a notice per AGENTS.md TUI invariant, enforced by test `the_catch_alls_other_arm_tells_the_user_instead_of_failing_silently` (tests.rs:1850).

## Q2 — /changes routing (all sites)

- Registry: SLASH_ITEMS "/changes" (command.rs:192-194) ✓
- Heads: COMMAND_HEADS "changes" (command.rs:463) ✓
- Parser: `"changes" => Command::Changes(tail)` (command.rs:588); `/changes all` → `Changes("all")` ✓
- Dispatch: not named in run_command's explicit arms (688-739) → catch-all (738) → run_workspace_command arm (1919-1921) → open_changes ✓
- Autocomplete: picker `slash_items`/`slash_candidates` derive from SLASH_ITEMS (picker.rs:360-404) ✓
- Palette: no /changes entry — palette is a curated subset; the enforced test only checks palette→parser direction (tests.rs:1812), so no violation (read).
- Verdict: **LIVE**. Minor (inferred): empty result opens the panel with no transcript notice; panel empty-state not verified from this file — see F3.

## Q3 — Palette + keybindings spot-check

- palette_entries (picker.rs:865-947): 8 entries; 7 slash commands all parse (guarded by test tests.rs:1812); 1 direct-action (ToggleFileTree, no command — deliberate, tested).
- key_ids used: `app.model.select`, `app.tree.toggle`, `app.tools.expand` — all present in TERMINAL (keybind.rs:243, 408, 257). No dead hints.
- TERMINAL table (keybind.rs:231-418): every id has ≥1 printable chord; `every_declared_chord_parses`-style tests cited in comments. No dead entries found (read).
- WINDOW table (keybind.rs:438-519): retained deliberately per AGENTS.md §2 inv.3 (desktop removed ADR-039, table kept as data). Documented, not dead.

## Findings (ranked by user value × fix size)

**F1 — `/skill` error paths quit the session** (commands.rs:978-983, 968-971). Unknown skill name → `push_notice` + `return true`; true = quit (run_command propagates run_session_command's bool). A typo in a skill name ends the session. Directly contradicts the comment at 991-995 ("must answer `false` — a `true` here ended the session"). Fix: `true`→`false` ×2. STATUS: read.
**F2 — `/theme <bad>` error names only two of three palettes** (commands.rs:1833-1835: "use `light` or `dark`…"). from_name accepts `high-contrast|contrast|hc` (theme.rs:149-156) and /theme advertises three (command.rs:243). A user trying the advertised third option and mistyping it is told only two exist. Fix: one-line message. STATUS: read.
**F3 — `/changes` with zero rows opens the panel silently** (commands.rs:1965-1981: no notice when list empty). (Inferred: panel may render its own empty state — verify in panel draw code before fixing.) Fix if confirmed: one `push_notice("changes: no checkpoints…")`. STATUS: read (code) / inferred (panel rendering).

## Note on /open

Terminal refusal (commands.rs:1906-1912) + SLASH_ITEMS description openly stating "no front end offers this yet; the terminal refuses by name" (command.rs:180-181) = honest refusal, not a silent no-op. No action.

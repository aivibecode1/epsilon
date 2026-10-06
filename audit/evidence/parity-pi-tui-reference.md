# Pi interactive terminal mode — behavioural reference inventory

## Header

| Fact | Value | Where it comes from |
|---|---|---|
| What Pi is | "Pi Agent Harness … including our self extensible coding agent"; `@earendil-works/pi-coding-agent` is the "Interactive coding agent CLI" | `references/pi-1.0.0/README.md:13`, `references/pi-1.0.0/README.md:17` |
| Version under audit | `1.0.0` (both `packages/coding-agent/package.json` and `packages/tui/package.json`) | `references/pi-1.0.0/packages/coding-agent/package.json`, `references/pi-1.0.0/packages/tui/package.json` |
| Licence, as stated by Pi's own LICENSE file | **MIT License, Copyright (c) 2025 Mario Zechner** | `references/pi-1.0.0/LICENSE:1`, `references/pi-1.0.0/LICENSE:3` |
| README licence claim | "MIT" | `references/pi-1.0.0/README.md:110` |
| Root repository | `references/pi-1.0.0/` (read-only input; never modified) | `AGENTS.md` §10, §11 |

### Paths read for this inventory

- `references/pi-1.0.0/README.md`, `references/pi-1.0.0/LICENSE`
- `references/pi-1.0.0/packages/coding-agent/src/modes/interactive/interactive-mode.ts` (7039 lines, read in full)
- `references/pi-1.0.0/packages/coding-agent/src/modes/interactive/{chat-viewport,external-editor,tui-renderer,model-search,model-catalog-refresh,bug-report,session-share}.ts` (read in full)
- `references/pi-1.0.0/packages/coding-agent/src/modes/interactive/components/*` (delegated; see each row)
- `references/pi-1.0.0/packages/coding-agent/src/modes/interactive/theme/*` (delegated)
- `references/pi-1.0.0/packages/coding-agent/src/core/{keybindings,slash-commands,settings-manager,footer-data-provider}.ts`
- `references/pi-1.0.0/packages/coding-agent/src/cli/{args,startup-ui}.ts`
- `references/pi-1.0.0/packages/coding-agent/docs/{tui,keybindings,slash-commands}.md`
- `references/pi-1.0.0/packages/tui/` (delegated)
- `references/pi-1.0.0/packages/coding-agent/test/interactive-*.test.ts`, `.../status-indicator.test.ts`, `.../chat-viewport.test.ts`, `.../footer-width.test.ts`, `.../session-selector-*.test.ts`, `.../model-selector.test.ts`, `.../theme-*.test.ts`, `.../settings-selector.test.ts`
- `references/pi-1.0.0/.pi/skills/interactive-testing.md` (how Pi itself verifies these behaviours, under tmux)

### How to run Pi to check a row

Pi's own agent rules document the verification procedure:

```
tmux new-session -d -s pi-test -x 80 -y 24
tmux send-keys -t pi-test "./pi-test.sh" Enter
sleep 3 && tmux capture-pane -t pi-test -p
```

`references/pi-1.0.0/.pi/skills/interactive-testing.md:11-17`

A row that says "observe: …" means: run that, send those keys, and read the captured pane.

### Citation check

Every `path:line` in this file was machine-validated against `references/pi-1.0.0`: the file must exist and the line number must be within it.

```
citations parsed: 613   resolvable: 613   distinct files cited: 95   problems: 0
```

That check proves each citation **points at a real line**, not that the line says what the row claims. Rows written from a delegated read were spot-verified by opening the cited lines directly; the sampling covered all four delegated areas.

**Counts.** 323 rows in the `P-###` series (P-001 to P-323, no gaps), 4 in `L-###` (the `tui.*` keybinding table), 12 in `T-###` (the TUI library's text-measurement core), 10 in `Y-###` (explicitly out of scope), 7 in `AA-###` (gaps). **356 rows total.**

---

## A. Startup, header, resources

### P-001 — Startup header shows logo, version and key hints
- **area:** startup
- **behaviour:** On a fresh interactive session a header is drawn containing the Pi logo, `v<version>`, and a compact one-line key hint list; terminals that cannot render the logo get a `Pi vX` wordmark line instead.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:996` (built at init), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1001-1006` (`supportsPiLogo()` branch building `piWordmark() … v${this.version}`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1051-1057` (the `BuiltInHeader` is constructed from those two functions)
- **how to observe it:** start Pi in an 80×24 tmux pane and `capture-pane` after 3 s.
- **notes:** the header is *lazy* — both collapsed and expanded bodies are functions, so a theme change recolours it without rebuilding (`interactive-mode.ts:991-993`, comment "bake theme colors into their text, so build them once the terminal reported its colors").

### P-002 — Ctrl+O toggles the startup help between a compact line and a full list
- **area:** startup / keybindings
- **behaviour:** The header starts as a compact single line (`Esc interrupt · Ctrl+C/Ctrl+D clear/exit · / commands · ! bash · Ctrl+O more`); the same key that toggles tool output expands it to a ~20-line instruction list.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1036-1043` (compact body), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1011-1035` (expanded body), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4470-4472` (`toggleToolOutputExpansion`), `packages/coding-agent/src/core/keybindings.ts:117` (`app.tools.expand` → `ctrl+o`)
- **how to observe it:** start Pi, press `Ctrl+O`; the hint line becomes the full list.
- **notes:** the *same* boolean (`toolOutputExpanded`) drives the startup header, the loaded-resources listing **and** every transcript row that implements `Expandable` (`interactive-mode.ts:4474-4490`). One toggle, three surfaces.

### P-003 — Clicking the logo plays an animation
- **area:** startup
- **behaviour:** Clicking the first two lines of the built-in header (columns 1–4) starts the Pi logo animation.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:254-263` (`BuiltInHeader.handleMouse` guard `event.y > 1 || event.x < 1 || event.x > 4`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1058` (`header.onLogoClick = … playPiLogoAnimation`)
- **how to observe it:** in fullscreen TUI mode, click the top-left logo.
- **notes:** the click handler is only attached when `showLogo` is true (`interactive-mode.ts:1001`, `1058`); a custom header from an extension replaces the built-in one and loses the easter egg.

### P-004 — `quietStartup` has three states, not a boolean
- **area:** startup / settings
- **behaviour:** `quietStartup: true` hides the whole header; `quietStartup: "header"` keeps the header but hides the loaded-resource listing; `false` (the default) shows both.
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1089-1092` (`value === true || value === "header" ? value : false`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1410-1417` (`shouldShowStartupHeader` / `shouldShowStartupDetails`), `packages/coding-agent/test/interactive-mode-status.test.ts:1245-1265` (both branches asserted)
- **how to observe it:** set `"quietStartup": "header"` in settings, restart, compare with `true`.
- **notes:** `--verbose` (or `InteractiveModeOptions.verbose`) forces *both* on regardless of the setting (`interactive-mode.ts:1411`, `1416`). A porter that models this as a bool loses the middle state.

### P-005 — Startup prints the model scope to stdout, not into the transcript
- **area:** startup / model
- **behaviour:** When the session has scoped models, Pi writes `Model scope: <id>[:<thinking>], … (Ctrl+P to cycle)` with `console.log` before the TUI starts, so it scrolls away in the terminal's own scrollback.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:940-953` (the `console.log` at line 952), `packages/coding-agent/src/core/keybindings.ts:108-111` (`app.model.cycleForward` → `ctrl+p`)
- **how to observe it:** start Pi with `--models` set; scroll up in a regular-mode terminal.
- **notes:** gated on `shouldShowStartupDetails()` (`interactive-mode.ts:940`), so `quietStartup: true` suppresses it. The hint is *omitted entirely* when the cycle-forward binding has been unbound (`interactive-mode.ts:948-951`).

### P-006 — Loaded resources are listed, grouped and abbreviated
- **area:** startup
- **behaviour:** A `[Context]`, `[Skills]`, `[Prompts]`, `[Extensions]` block lists what the session loaded, grouped into `project` / `user` / `path`, with package sources as sub-headings.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1759-1953` (`showLoadedResources`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1648-1680` (`formatScopeGroups`), `packages/coding-agent/test/interactive-mode-status.test.ts:1141-1169` (exact expected output snapshot)
- **how to observe it:** create `.pi/extensions/foo/index.ts` in a project and restart.
- **notes:** expanded form shows *absolute* paths; compact form shows the last path segment, `index.ts`/`index.js` stripped to the parent dir, and enough parent folders added to make local labels unique (`interactive-mode.ts:1504-1566`; asserted at `interactive-mode-status.test.ts:795-839`, `841-895`). npm-sourced extensions get a `pkg:path` label, and a sibling package in the same `node_modules` renders as `pkg:../sibling` (`interactive-mode-status.test.ts:1064-1099`).

### P-007 — Resource conflicts render as diagnostics blocks
- **area:** startup / diagnostics
- **behaviour:** Duplicate or broken skills, prompts, extensions and themes render under `[Skill conflicts]`, `[Prompt conflicts]`, `[Extension issues]` and `[Theme conflicts]`, with a ✓ on the winning path and ✗ on each `(skipped)` loser.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1706-1757` (`formatDiagnostics`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1901-1952` (the four diagnostic sections), `packages/coding-agent/test/interactive-mode-status.test.ts:1267-1282` (diagnostics still shown on quiet startup when requested)
- **how to observe it:** define two skills with the same `name` in one project.
- **notes:** diagnostics are shown even under `quietStartup: true` *if* the caller passes `showDiagnosticsWhenQuiet: true` — which `bindCurrentSessionExtensions` always does (`interactive-mode.ts:2031`). The listing itself is still suppressed (`interactive-mode-status.test.ts:1230-1243`).

### P-008 — An extension command that shadows a built-in is reported, not silently dropped
- **area:** slash commands / diagnostics
- **behaviour:** If an extension registers a command whose name equals a built-in, Pi prints a diagnostic; if the extension also has an `invocationName` it says the command is available under that alias instead.
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:699-712` (`getBuiltInCommandConflictDiagnostics`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:784-791` (extension commands with a colliding name are filtered out of autocomplete)
- **how to observe it:** register `registerCommand({ name: "model", … })` in an extension, run `/reload`, read `[Extension issues]`.
- **notes:** the diagnostic text differs for the aliased and unaliased case (`interactive-mode.ts:707-709`). Autocomplete uses `invocationName`, not `name` (`interactive-mode.ts:788`).

### P-009 — "What's New" changelog block on startup, once per version
- **behaviour:** When the changelog has entries newer than the last-seen version, Pi renders a bordered `What's New` markdown block in the transcript at startup.
- **area:** startup
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:832-860` (`showStartupNoticesIfNeeded`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1319-1344` (`getChangelogForDisplay`), `packages/coding-agent/src/core/settings-manager.ts:1131-1133` (`collapseChangelog` default `false`)
- **how to observe it:** delete `lastChangelogVersion` from settings and restart.
- **notes:** three distinct rules a porter will get wrong. (a) A **resumed** session never shows it (`interactive-mode.ts:1321-1323`). (b) A **fresh install** (no `lastChangelogVersion`) records the version and shows *nothing* (`interactive-mode.ts:1329-1334`). (c) With `collapseChangelog: true` the block collapses to `Updated to vX. Use /changelog to view full changelog.`, and the version number is regex-scraped out of the changelog text, falling back to the running version (`interactive-mode.ts:846-850`).

### P-010 — Untrusted-project warning is written into the transcript
- **behaviour:** If the project has trust-requiring resources and no trust decision is saved, a warning line tells the user project `.pi` resources and packages are ignored and to use `/trust`.
- **area:** permissions / startup
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4145-4164` (`renderProjectTrustWarningIfNeeded`, text at line 4158), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4134` (called from `renderInitialMessages`)
- **how to observe it:** drop a `.pi/extensions/x.ts` into an untrusted project and start Pi.
- **notes:** the guard is `isProjectTrusted() || !hasTrustRequiringProjectResources(cwd)` — a project with no `.pi` resources never sees this (`interactive-mode.ts:4146`). The trust *dialog* is a separate CLI/startup concern, not this line.

### P-011 — The system theme waits for the terminal's reported colours
- **behaviour:** Pi queries the terminal for its foreground/background colours (OSC 10/11) and derives the "system" theme from them; until the reply arrives (or 100 ms elapse) the theme renders in grayscale.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:991-993` (`await this.themeController.waitForTerminalColors()`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1093-1097` (`onThemeChange` → full invalidate + editor border recolour), `packages/coding-agent/src/cli/startup-ui.ts:88-89` (`markTerminalColorsPending()`; "starts in grayscale until the terminal reports its colors")
- **how to observe it:** start Pi in a terminal with a light background and watch the first frame flip.
- **notes:** Pi re-queries colours on a **theme file watcher** (`interactive-mode.ts:1093`) and disables auto-sync only at shutdown (`interactive-mode.ts:7026`).

### P-012 — `fd` and `rg` are ensured at startup, after the TUI is already drawing
- **behaviour:** Pi downloads `fd` and `rg` if they are missing, printing progress lines into the transcript, and does so *after* `ui.start()` so a slow download never looks like a frozen startup.
- **area:** startup / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:987` (`this.ui.start()`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1075-1079` (`Promise.all([ensureTool("fd"), ensureTool("rg")])`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3742-3753` (`showManagedToolStatus`)
- **how to observe it:** clear `PATH` of `rg`, start Pi, watch the transcript.
- **notes:** only `fd` is stored (`this.fdPath`) — `rg` is only needed by the grep tool and bash, not by autocomplete (`interactive-mode.ts:1079`). The **full** key handler set is only installed *after* this completes (`interactive-mode.ts:1082`), which is what makes P-013 possible.

### P-013 — Input is accepted before startup finishes, then restored
- **behaviour:** Typing and pressing Enter while startup is still running does not lose the text: the editor is refilled and a status line says `Startup is still in progress`.
- **area:** composer
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3135-3138` (`handleStartupSubmit`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:980-983` (only `app.clear`, `app.exit` and submit are wired during startup), `packages/coding-agent/test/interactive-mode-startup-input.test.ts:58-68`
- **how to observe it:** on a cold start with a slow `fd` download, type a prompt and Enter immediately.
- **notes:** prompts submitted before `run()` installs its input callback go into `pendingUserInputs` and are drained by `getUserInput()` (`interactive-mode.ts:3332-3336`, `4166-4178`; asserted at `interactive-mode-startup-input.test.ts:70-89`).

---

## B. Layout, renderer and viewport

### P-014 — Two renderer modes: main screen and alt screen
- **behaviour:** Pi has two entirely different renderers. `regular` is a `TuiMainScreen` that plays nicely with the terminal's own scrollback and does not take the mouse; `fullscreen` is a `TuiAltScreen` that owns the whole screen, routes mouse events to components, and supports search, selection and copy.
- **area:** layout
- **evidence:** `packages/coding-agent/src/modes/interactive/tui-renderer.ts:22-50` (`createInteractiveTui` — the two branches), `packages/coding-agent/test/interactive-tui.test.ts:52-80` (regular mode must **not** emit `ESC[?1049h`, fullscreen must)
- **how to observe it:** `/settings` → `tui-mode`; then check whether the alternate-screen sequence appears.
- **notes:** **the default is `fullscreen`, not `regular`** — `getTuiMode()` returns `fullscreen` for anything that is not the literal string `"regular"` (`packages/coding-agent/src/core/settings-manager.ts:1348-1350`). A porter defaulting to "regular" will not match.

### P-015 — TUI mode can be switched live, but not while an overlay is open
- **behaviour:** Changing `tui-mode` in `/settings` rebuilds the renderer in place, keeping the component tree, the focused component and the main screen's render state; if any overlay entries exist the switch is refused with `Close active overlays before changing TUI mode` and the settings row is reverted.
- **area:** layout / settings
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:879-930` (`switchTuiMode`; the refusal at line 882), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5029-5038` (`onTuiModeChange`, message at line 5032), `packages/coding-agent/test/interactive-tui.test.ts:111-181`
- **how to observe it:** open an extension overlay, then change `tui-mode` in `/settings`.
- **notes:** the component list is copied out of the old renderer and re-mounted (`interactive-mode.ts:884`, `915`); the main screen captures/restores `TuiMainScreenRenderState` so a scroll position survives (`interactive-mode.ts:890-891`, `910-912`).

### P-016 — Fullscreen layout is a scrolling transcript plus a fixed input dock
- **behaviour:** In fullscreen the transcript is a `ScrollView` that follows the end by default and chains overscroll; below it a fixed dock stacks pending messages, the status row, widgets-above, the editor (minimum 3 lines), widgets-below and the footer.
- **area:** layout
- **evidence:** `packages/coding-agent/src/modes/interactive/chat-viewport.ts:22-45` (`createChatViewport`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:957-970` (the call, with the scrollbar setting and theme-coloured track/thumb)
- **how to observe it:** switch to fullscreen, resize the terminal, scroll with the wheel.
- **notes:** in **regular** mode there is no viewport at all — the same seven containers are added directly as children of the screen (`interactive-mode.ts:971-979`, versus `mountInteractiveTui` at `interactive-mode.ts:862-868` which only attaches the layout root when `isViewportTUI(tui)`). The editor's `minSize: 3` is the floor that keeps the composer visible on a short terminal.

### P-017 — The terminal title tracks the session name and cwd
- **behaviour:** The terminal title is `Pi - <session name> - <cwd basename>`, dropping the name when the session has none.
- **area:** layout / sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1119-1127` (`updateTerminalTitle`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3429-3433` (`session_info_changed` triggers it), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1160-1165` (Windows: npm's console-title overwrite is undone after the startup package check)
- **how to observe it:** `/name foo`, watch the tab/window title.
- **notes:** the Windows-specific restore at `interactive-mode.ts:1162-1164` exists because checking extension package versions spawns npm, which overwrites the console title. A port that loses this regresses on Windows only.

### P-018 — Terminal progress indicator during turns and compaction
- **behaviour:** With `showTerminalProgress` on, Pi sets the terminal's progress state on `turn_start`, `compaction_start` and at startup while streaming, and clears it on `agent_end` / `compaction_end`; it is restored after a renderer switch.
- **area:** layout / status
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3365-3368` (turn_start), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3590-3593` (agent_end), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:922-928` (restored after `switchTuiMode`), `packages/coding-agent/src/core/settings-manager.ts:1335-1337` (default **false**)
- **how to observe it:** enable `terminal.showTerminalProgress`, send a prompt, watch the OS taskbar/dock indicator.
- **notes:** default is off (`?? false`, `settings-manager.ts:1336`). `stop()` clears it unconditionally at `interactive-mode.ts:7022-7024`.

### P-019 — `clearOnShrink` controls whether a residual idle row is kept
- **behaviour:** When a status indicator is cleared, an `IdleStatus` row is left in place only in regular mode *and* only when `clearOnShrink` is true; otherwise the status container is emptied and the lines scroll away.
- **area:** layout / status
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2277-2296` (`clearStatusIndicator`, the guard at 2288-2295), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2039-2066` (`applyRuntimeSettings`, line 2055), `packages/coding-agent/test/interactive-tui.test.ts:457-482` (regular → 1 child, fullscreen → 0)
- **how to observe it:** toggle `terminal.clearOnShrink` and watch what remains under the last message after a turn finishes.
- **notes:** the status indicator is normally **embedded in the editor** rather than drawn on its own row (`interactive-mode.ts:2257-2275`), so it consumes no extra height. That opt-in is per-editor (`isWorkingStatusEditor`, `interactive-mode.ts:220-227`); a custom extension editor that does not implement it silently gets a separate status row (`interactive-tui.test.ts:392-455`).

### P-020 — Extension widgets render above or below the editor, capped at 10 lines
- **behaviour:** An extension can place persistent content immediately above or below the composer; a string-array widget longer than 10 lines is truncated with a `... (widget truncated)` line.
- **area:** extensions / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2350-2390` (`setExtensionWidget`, truncation at 2375-2380), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2438` (`MAX_WIDGET_LINES = 10`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2450-2471` (leading spacer only above)
- **how to observe it:** call `ctx.ui.setWidget("k", ["a","b"])` from an extension command.
- **notes:** the cap exists specifically "to prevent viewport overflow" (`interactive-mode.ts:2437`). A widget factory can return a live component instead of strings, with no line cap (`interactive-mode.ts:2382-2385`).

### P-021 — An extension can replace the header, the footer or the editor wholesale
- **behaviour:** `ctx.ui.setFooter`, `ctx.ui.setHeader` and `ctx.ui.setEditorComponent` swap the built-in component for one the extension builds; passing `undefined` restores the built-in one, preserving editor text across the swap.
- **area:** extensions / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2476-2498` (footer), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2503-2542` (header, swapped *in place* by index), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2816-2891` (editor; text saved at 2820, restored at 2877)
- **how to observe it:** an extension sets a custom footer, then clears it.
- **notes:** the custom editor inherits `onSubmit`, `onChange`, border colour, padding, autocomplete max-visible, autocomplete provider, and — if it duck-types as a `CustomEditor` — the whole `actionHandlers` map (`interactive-mode.ts:2830-2872`). `setHeader` is a no-op if the header has not been built yet (`interactive-mode.ts:2505-2507`).

---

## C. Composer and editor

### P-022 — `!` puts the composer into bash mode and recolours the border
- **behaviour:** As soon as the editor text (ignoring leading whitespace) starts with `!`, the editor border changes to the bash-mode colour; deleting the `!` restores the thinking-level colour.
- **area:** composer
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3064-3070` (`onChange` sets `isBashMode`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4429-4438` (`updateEditorBorderColor`, bash branch at 4430-4431)
- **how to observe it:** type `!ls` and watch the border colour change before you press Enter.
- **notes:** the trigger is `text.trimStart().startsWith("!")` (`interactive-mode.ts:3066`), so `  !ls` also triggers. Otherwise the border colour encodes the **thinking level** (`interactive-mode.ts:4433-4434`) and changes whenever the level or model changes.

### P-023 — `!cmd` runs a shell command in the transcript; `!!cmd` excludes it from model context
- **behaviour:** A submitted line beginning with `!` is executed as a shell command and rendered as a `BashExecutionComponent`; `!!` marks the result as excluded from what the model will see.
- **area:** bash / files
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3287-3303` (the `!` / `!!` split at lines 3289-3290), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6917-7008` (`handleBashCommand`, `excludeFromContext` threaded at 6988)
- **how to observe it:** type `!echo hi`, Enter; then `!!echo secret`, Enter, and inspect the next prompt's context.
- **notes:** (a) the text is added to editor history *before* execution (`interactive-mode.ts:3297`). (b) If a bash command is already running, Pi refuses with `A bash command is already running. Press Esc to cancel it first.` and **puts the text back** (`interactive-mode.ts:3292-3296`). (c) Extensions get a `user_bash` event first and can return a result or supply custom operations; if the handler throws, Pi does **not** fall back to local execution (`interactive-mode.ts:6922-6932`).

### P-024 — Bash output is deferred into the pending area while the agent is streaming
- **behaviour:** A bash card started while the agent is streaming appears above the composer; it is moved into the transcript when the next message is submitted.
- **area:** bash / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6966-6976` (`isDeferred` → `pendingMessagesContainer` and `pendingBashComponents`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4774-4780` (`flushPendingBashComponents`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3330` (called on normal submission)
- **how to observe it:** while a response is streaming, type `!date` and Enter; submit a message and see the card move down.
- **notes:** `flushPendingBashComponents` is called **only** on the normal-submission path (`interactive-mode.ts:3330`), not on the steer/compaction paths — so a deferred card can linger there.

### P-025 — Escape has a four-way priority
- **behaviour:** One Escape key means, in order: abort the stream (restoring queued messages to the editor); abort a running bash command; leave bash mode by clearing the editor; otherwise, if the editor is empty, arm a double-Escape timer.
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3011-3037` (`onEscape`, the four branches at 3012, 3014, 3016, 3020)
- **how to observe it:** stream a response, press Escape; then press Escape again with an empty editor.
- **notes:** during compaction and during auto-retry the main Escape handler is **replaced**, not chained, and restored afterwards (`interactive-mode.ts:3614-3617`, `3627-3630`; `3677-3680`, `3690-3693`). A port that layers these instead of replacing them will abort the wrong thing.

### P-026 — Double-Escape within 500 ms opens the tree (default) or the fork picker
- **behaviour:** With an empty editor, pressing Escape twice within 500 ms opens `/tree`; setting `doubleEscapeAction` to `fork` opens `/fork` instead, and `none` disables it.
- **area:** keybindings / sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3021-3035` (`now - this.lastEscapeTime < 500`), `packages/coding-agent/src/core/settings-manager.ts:1446-1448` (`?? "tree"` — the default is **tree**, not fork)
- **how to observe it:** press Escape twice on an empty editor; then set `doubleEscapeAction: "none"` and repeat.
- **notes:** the timer is reset to `0` after firing so a third Escape does not immediately re-open it (`interactive-mode.ts:3031`).

### P-027 — Ctrl+C clears the editor; a second Ctrl+C within 500 ms exits
- **behaviour:** The first Ctrl+C clears the composer; a second one inside 500 ms quits Pi.
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4189-4197` (`handleCtrlC`), `packages/coding-agent/src/core/keybindings.ts:94` (`app.clear` → `ctrl+c`)
- **how to observe it:** type text, Ctrl+C (text gone), Ctrl+C again (exits).
- **notes:** the header advertises both forms separately: `Ctrl+C to clear` and `Ctrl+C twice to exit` (`interactive-mode.ts:1015-1016`).

### P-028 — Ctrl+D exits only when the editor is empty
- **behaviour:** Ctrl+D quits Pi; with text in the composer it is a no-op (the binding is only reached when the editor is empty).
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4199-4202` ("Only called when editor is empty (enforced by CustomEditor)"), `packages/coding-agent/src/core/keybindings.ts:95` (`app.exit` → `ctrl+d`)
- **how to observe it:** Ctrl+D on empty input vs. with text.
- **notes:** the *enforcement* lives in the editor component, not in interactive mode — a replacement editor must reproduce it or the exit becomes destructive.

### P-029 — Ctrl+Z suspends, except on Windows
- **behaviour:** Ctrl+Z stops the TUI and sends `SIGTSTP` to the process group; a `SIGCONT` handler restarts the TUI and forces a full redraw. On Windows it prints `Suspend to background is not supported on Windows` instead.
- **area:** keybindings / platform
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4351-4386` (Windows branch at 4352-4355; `process.kill(0, "SIGTSTP")` at 4380), `packages/coding-agent/test/interactive-mode-suspend.test.ts:65-113` (asserts the keep-alive interval, the `SIGCONT` restart and `requestRender(true)`), `packages/coding-agent/src/core/keybindings.ts:96-99` (default keys `[]` on `win32`)
- **how to observe it:** Ctrl+Z then `fg` in a POSIX shell; on Windows press Ctrl+Z and read the status line.
- **notes:** three things a porter will miss. (a) A `setInterval` keep-alive holds the event loop open while suspended, because stopping the TUI can leave Node with no ref'd handles and the process would exit before `SIGCONT` (`interactive-mode.ts:4357-4360`). (b) `SIGINT` is ignored while suspended so Ctrl+C does not kill the backgrounded job (`4363-4365`). (c) `SIGHUP` is *not* registered as a hard exit — it triggers a graceful shutdown (`interactive-mode.ts:4307-4310`, `4313-4320`). WSL counts as non-Windows for keybinding purposes (`keybindings.ts:62-67`).

### P-030 — Alt+Enter queues a follow-up while streaming; it is plain Enter when idle
- **behaviour:** While the agent is working, `app.message.followUp` sends the composer text as a follow-up that waits for the turn to finish. When idle it behaves exactly like Enter.
- **area:** composer / message queue
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4388-4418` (streaming branch 4406-4412; idle fallback 4414-4417), `packages/coding-agent/src/core/keybindings.ts:134-137` (`alt+enter`, or `ctrl+q` on Windows/WSL)
- **how to observe it:** start a response, type a correction, press Alt+Enter — the `Follow-up:` line appears above the composer.
- **notes:** empty text is ignored (`interactive-mode.ts:4389-4390`). The Windows/WSL rebinding to `ctrl+q` exists because Alt+Enter is not reliably deliverable there (`keybindings.ts:135`, `keybindings.md:165`).

### P-031 — Enter while streaming steers the running turn
- **behaviour:** Submitting while the agent is streaming sends the text with `streamingBehavior: "steer"`, so it redirects the in-flight turn rather than queueing behind it.
- **area:** composer / message queue
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3317-3326` (the `isStreaming` branch, `streamingBehavior: "steer"` at 3322)
- **how to observe it:** mid-response, type "actually use Rust instead" and press Enter; the transcript shows a `Steering:` row.
- **notes:** the two submission keys are genuinely different semantics — Enter steers, Alt+Enter follows up. Extension commands inside either path execute immediately rather than being queued (`interactive-mode.ts:3307-3313`, `4393-4401`).

### P-032 — Enter during compaction queues; the queue is flushed in order when compaction ends
- **behaviour:** While compaction runs, submitted text is held in a separate compaction queue and shown with `Queued message for after compaction`; when compaction finishes the first non-extension message becomes the prompt and the rest are steered/followed up behind it.
- **area:** composer / compaction
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3306-3315` (queueing branch), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4676-4682` (`queueCompactionMessage`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4694-4771` (`flushCompactionQueue`, ordering at 4731-4765)
- **how to observe it:** `/compact`, type three messages while the status row shows compaction, watch them flush in order.
- **notes:** (a) If a retry is pending (`willRetry`), every queued message is re-queued for the retry turn rather than started as a prompt (`interactive-mode.ts:4715-4728`). (b) Extension commands queued before the first real message are executed first (`4741-4747`). (c) A failure restores the whole queue and prints `Failed to send queued message(s): …` — the messages are never silently dropped (`4703-4712`).

### P-033 — `app.message.dequeue` pulls every queued message back into the composer
- **behaviour:** One key clears both queues (session and compaction), joins the messages with a blank line, prepends them to whatever is already in the editor, and reports the count.
- **area:** composer / message queue
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4420-4427` (`handleDequeue`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4655-4674` (`restoreQueuedMessagesToEditor`), `packages/coding-agent/src/core/keybindings.ts:138-141` (`alt+up`, or `alt+q` on Windows/WSL)
- **how to observe it:** queue two steering messages, press the dequeue key, read the composer.
- **notes:** the same routine with `{ abort: true }` is what Escape uses (`interactive-mode.ts:3013`, `4670-4672`), so aborting never loses queued text — that is the point of the ordering (`interactive-mode-compaction.test.ts:232-247`, regression for #9340). An empty queue prints `No queued messages to restore`.

### P-034 — Ctrl+X copies the selection in fullscreen, otherwise the last assistant message
- **behaviour:** In fullscreen with `fullscreenCopyOnSelect: false` and an active selection, Ctrl+X copies the selection; otherwise it copies the last assistant message. In fullscreen the confirmation is a screen flash (`Copied!`); in regular mode it is a status line.
- **area:** keybindings / clipboard
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6556-6585` (`handleCopyCommand`; selection branch 6559-6567; flash at 6577-6579), `packages/coding-agent/test/interactive-tui.test.ts:226-363` (all four combinations asserted against the rendered viewport), `packages/coding-agent/src/core/settings-manager.ts:1379-1381` (`fullscreenCopyOnSelect` defaults **true**)
- **how to observe it:** fullscreen, drag-select text, Ctrl+X; then Ctrl+X with no selection.
- **notes:** when no assistant message exists the result is an error, not a silent success: `No agent messages to copy yet.` (`interactive-mode.ts:6570-6572`). With copy-on-select **on**, Ctrl+X still copies the last assistant message even with a selection live (`interactive-tui.test.ts:269-309`) — that surprises people.

### P-035 — Ctrl+V pastes files, then images, then text
- **behaviour:** One paste key tries, in order: clipboard **file paths** (inserted as text, one per line, shell-quoted in bash mode); a clipboard **image** (written to a temp file and its path inserted); then clipboard **text**.
- **area:** keybindings / files
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3093-3133` (`handleClipboardPaste`; file paths 3095-3109, image 3112-3123, text 3125-3129), `packages/coding-agent/src/core/keybindings.ts:142-145` (`ctrl+v`, `alt+v` on Windows/WSL)
- **how to observe it:** copy a file in the OS file manager, focus Pi, Ctrl+V.
- **notes:** security-relevant detail: a clipboard path containing a control character is **rejected** with `Clipboard file path contains control characters` (`interactive-mode.ts:3097-3099`). Spaces are inserted around pasted paths when the cursor sits between non-whitespace (`3105-3106`). Pasted images are written as `pi-clipboard-<uuid>.<ext>` in the OS temp dir (`3114-3118`).

### P-036 — Ctrl+G hands the composer to an external editor
- **behaviour:** The TUI stops, `$EDITOR`-style command opens on a temp file holding the composer text, and the result replaces the composer text only on a zero exit code.
- **area:** composer / external editor
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4509-4525` (`handleOpenExternalEditor`; text replaced only when `status === "complete"` at 4518-4520), `packages/coding-agent/src/modes/interactive/external-editor.ts:14-45` (temp dir `pi-editor-`, file `prompt.md`, non-zero exit → `{ status: "failed" }`), `packages/coding-agent/src/core/settings-manager.ts:1054-1064` (resolution order: `externalEditor` setting → `$VISUAL` → `$EDITOR` → `notepad` on Windows → `nano`)
- **how to observe it:** type a long prompt, Ctrl+G, edit in vim, `:wq`.
- **notes:** the child is spawned **asynchronously, never `spawnSync`** — the source comments explain that synchronous spawns race vim/nvim for the console input buffer on Windows (`external-editor.ts:22-24`). A non-zero exit leaves your text alone (never lost).

### P-037 — Shift+Tab cycles the thinking level and reports the new one
- **behaviour:** One key steps the thinking level; if the current model has no reasoning support Pi says `Current model does not support thinking` instead.
- **area:** model / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4440-4449` (`cycleThinkingLevel`), `packages/coding-agent/src/core/keybindings.ts:100-103` (`shift+tab`)
- **how to observe it:** press Shift+Tab repeatedly and read the status line and the editor border colour.
- **notes:** the border colour changes with the level (`interactive-mode.ts:4433-4434`), so the shift is visible without reading the status line. `/thinking` persists a default; Shift+Tab does not (`interactive-mode.ts:5084-5093`, `persist` flag).

### P-038 — Ctrl+P / Ctrl+Shift+P cycle models and announce the switch
- **behaviour:** `app.model.cycleForward` / `cycleBackward` step through the scoped (or all available) models and print `Switched to <name> (thinking: <level>)`; with a single model they report `Only one model in scope` or `Only one model available`.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4451-4468` (`cycleModel`, messages at 4455 and 4460-4462), `packages/coding-agent/src/core/keybindings.ts:108-115` (`ctrl+p`; `shift+ctrl+p`, or `alt+p` on Windows/WSL)
- **how to observe it:** press Ctrl+P a few times.
- **notes:** the two messages are deliberately different — "in scope" when `/scoped-models` narrowed the list, "available" otherwise (`interactive-mode.ts:4455`). The thinking suffix is only shown for a reasoning model with a non-off level (`4460-4461`).

### P-039 — Ctrl+O collapses and expands every transcript row that supports it
- **behaviour:** One key flips a single global expansion flag applied to the header, the loaded-resource sections, and every `Expandable` child already in the transcript; the state is reported as `Tool output: expanded` / `Tool output: collapsed`.
- **area:** transcript / display
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4470-4490` (`setToolsExpanded`), `packages/coding-agent/test/interactive-mode-status.test.ts:146-169` (asserts header + loaded-resources + chat children all receive it)
- **how to observe it:** produce a long tool result, press Ctrl+O twice.
- **notes:** it iterates only `loadedResourcesContainer` and `chatContainer` (`interactive-mode.ts:4482`) — not the pending-messages area — and it is duck-typed via `setExpanded` (`interactive-mode.ts:229-231`), so any component with that method participates.

### P-040 — Ctrl+T hides thinking blocks, and the choice is persisted
- **behaviour:** Ctrl+T toggles thinking-block visibility across the transcript and writes the choice to global settings immediately; the status line reports `Thinking blocks: hidden` / `visible`.
- **area:** transcript / display
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4502-4507` (`toggleThinkingBlockVisibility`, `settingsManager.setHideThinkingBlock`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4493-4500` (`updateThinkingBlockVisibility` walks live components), `packages/coding-agent/src/core/settings-manager.ts:1046-1048` (default `false`)
- **how to observe it:** run a reasoning model, press Ctrl+T.
- **notes:** the persisted setting lands in the **global** settings file (`settings-manager.ts:1066-1070`), not the project one. Streaming components pick the state up at construction (`interactive-mode.ts:3451`).

---

## D. Slash commands and autocomplete

### P-041 — The built-in slash-command table
- **behaviour:** 24 built-in commands are offered: `settings`, `model`, `tree`, `thinking`, `scoped-models`, `export`, `import`, `share`, `bug`, `copy`, `name`, `session`, `changelog`, `hotkeys`, `fork`, `clone`, `trust`, `login`, `logout`, `new`, `compact`, `resume`, `reload`, `quit`.
- **area:** slash commands
- **evidence:** `packages/coding-agent/src/core/slash-commands.ts:19-44` (`BUILTIN_SLASH_COMMANDS`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3140-3338` (`setupEditorSubmitHandler` — the dispatch chain), `packages/coding-agent/docs/slash-commands.md:9-52` (the documented table)
- **how to observe it:** type `/` in the composer.
- **notes:** the table in `core/slash-commands.ts` is the single source for both autocomplete and conflict detection (`interactive-mode.ts:716-720`, `700`); the docs list `/llama` too, which comes from a built-in *extension*, not this table (`docs/slash-commands.md:17`).

### P-042 — Four hidden commands that work but are not advertised
- **behaviour:** `/debug`, `/arminsayshi` and `/dementedelves` are handled but do not appear in autocomplete or the documented table.
- **area:** slash commands / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3261-3275` (dispatch), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6860-6891` (`handleDebugCommand` writes a render log), `packages/coding-agent/src/core/slash-commands.ts:19-44` (their absence from the table)
- **how to observe it:** type `/debug` and Enter; a `✓ Debug log written` line with a file path appears.
- **notes:** `/debug` dumps every rendered line with its visible width plus the full message list as JSONL, and is also bound globally through `ui.onDebug` (`interactive-mode.ts:3048`, dump at `6866-6884`). The other two draw easter-egg components (`6893-6903`).

### P-043 — Autocomplete composes built-ins, prompt templates, extension commands and skills
- **behaviour:** The `/` menu merges four sources in one list: built-in slash commands, prompt templates, extension commands (excluding names that collide with built-ins), and `/skill:<name>` entries when skill commands are enabled.
- **area:** composer / autocomplete
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:776-811` (templates, extensions, `skillCommands` at 796-805, the merged `CombinedAutocompleteProvider` at 807-811), `packages/coding-agent/docs/slash-commands.md:54-59` ("Skills are available as `/skill:name` when skill commands are enabled")
- **how to observe it:** type `/` in a project with `.pi/prompts/` and `.pi/skills/`.
- **notes:** the provider is rebuilt (not mutated) whenever skill-command enablement changes (`interactive-mode.ts:4899-4902`) and on `/reload` (`6412`). Extension wrappers can stack on top and contribute extra trigger characters (`interactive-mode.ts:814-830`; `interactive-mode-status.test.ts:373-401`).

### P-044 — `/model`, `/thinking` and `/login` complete their arguments
- **behaviour:** Three commands carry their own argument completions: `/model` fuzzy-matches `provider/id` labels, `/thinking` matches the model's available levels, `/login` matches provider ids and names and shows the auth methods as the description.
- **area:** composer / autocomplete
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:722-773` (the three `getArgumentCompletions`), `packages/coding-agent/test/interactive-mode-status.test.ts:456-517` (`/login subscription anthrop` → `Anthropic · subscription/API key`; `/login radius` → `Radius · account`)
- **how to observe it:** type `/login ` then `anth`.
- **notes:** when a provider supports several auth types they are merged into one row, ordered **oauth before api_key** (`interactive-mode.ts:369`, `389-392`). "subscription" vs "account" depends on `provider.auth.oauth.isSubscription` (`interactive-mode.ts:5722`, `405-417`) — a porter will get the wording wrong from memory.

### P-045 — Autocomplete descriptions carry a scope tag
- **behaviour:** Non-built-in completion descriptions are prefixed with a bracketed source tag: `[u]`, `[p]`, `[t]` for user/project/temporary scope, `u:npm:<pkg>` for npm packages, and `p:git:<host>/<path>@<ref>` for git sources.
- **area:** composer / autocomplete
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:665-689` (`getAutocompleteSourceTag`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:691-697` (`prefixAutocompleteDescription`)
- **how to observe it:** type `/` with a git-sourced extension installed and read the tag.
- **notes:** built-in extension commands are deliberately **untagged**, exactly like built-ins (`interactive-mode.ts:666-668`).

### P-046 — `/model <exact-id>` selects without opening the picker
- **behaviour:** `/model provider/id` resolves an exact match against the scoped or cached model list and switches immediately; only if it misses does Pi refresh the catalogs (15 s budget) and then open the selector pre-filled.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5116-5138` (`handleModelCommand`; selector fallback at 5137), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5140-5172` (`findExactModelMatch`, timeout at 5151-5154)
- **how to observe it:** `/model openai/gpt-4o`.
- **notes:** three distinct degradation messages: `Refreshing model catalogs…`, `Model refresh timed out; searching cached models.`, and `Could not refresh <providers>; searching cached models.` (`interactive-mode.ts:5148`, `5158`, `5160`). Concurrent refreshes are de-duplicated per runtime with independent cancellation (`model-catalog-refresh.ts:13-51`).

### P-047 — `/scoped-models` chooses which models Ctrl+P cycles
- **behaviour:** `/scoped-models` opens a searchable checklist; changing the selection immediately rescopes the session's cycle list, and Ctrl+S persists the patterns to settings.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5302-5422` (`showModelsSelector`; `onPersist` at 5362-5370; catalog refresh at 5377-5411), `packages/coding-agent/docs/keybindings.md:184-194` (the in-selector bindings), `packages/coding-agent/src/core/keybindings.ts:186-209` (`app.models.*`)
- **how to observe it:** `/scoped-models`, deselect some, then press Ctrl+P.
- **notes:** persisting **all** models clears the setting rather than writing a list of every id (`interactive-mode.ts:5363-5368`), and if the selection ends up empty or covers everything the session scope is cleared to `[]` (`5326-5338`). The refresh status line is one of `Model catalogs refreshed.`, `Model refresh timed out; showing cached models.`, `Could not refresh <providers>; showing cached models.` (`5389-5398`).

### P-048 — `/thinking <level>` sets it; `/thinking` alone opens a picker that can also persist
- **behaviour:** With an argument, `/thinking <level>` sets the level for this session (case-insensitive, exact match); with no argument it opens a picker where a separate action saves the choice as the default.
- **area:** model / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5067-5082` (`handleThinkingCommand`, error text at 5077), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5095-5114` (`showThinkingSelector`, `persist` path at 5109-5110)
- **how to observe it:** `/thinking bogus` → the error lists the available levels.
- **notes:** the two outcomes read differently: `Thinking level: <level>` for the session, `Default thinking level: <level>` for the persisted default (`interactive-mode.ts:5089`). Per-model overrides set in `/settings` take precedence and revert to the global default when removed (`4922-4942`).

---

## E. Transcript and streaming

### P-049 — Streaming assistant text updates a single component in place
- **behaviour:** On `message_start` for an assistant message a component is appended to the transcript; each `message_update` re-renders that same component rather than appending lines.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3448-3461` (component created), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3464-3468` (updated in place)
- **how to observe it:** send a prompt and watch text grow without flicker or duplicated rows.
- **notes:** the component is constructed with the *current* theme, hide-thinking state, hidden-thinking label, output pad and markdown transformers (`interactive-mode.ts:3449-3456`) — all settings-driven.

### P-050 — Tool cards appear as the model calls them, before the tool runs
- **behaviour:** A `toolCall` content block creates its card immediately during streaming; a later `tool_execution_start` re-uses the same card rather than making a second one.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3469-3493` (card created on the content block; `pendingTools` map at 3486), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3545-3569` (`tool_execution_start` reuses it)
- **how to observe it:** send a prompt that makes several tool calls; cards appear in call order.
- **notes:** (a) **Nested** tool calls (a tool calling another tool, e.g. inside a codemode script) are deliberately *not* given their own row — they are shown inside the parent's row (`interactive-mode.ts:3546-3547`). (b) Live partial results stream into the card via `tool_execution_update` (`3571-3578`). (c) The args are incomplete while streaming, so diff computation for edit tools only fires at `message_end` via `setArgsComplete()` (`3527-3530`).

### P-051 — An aborted or errored turn rewrites every in-flight tool card with the error
- **behaviour:** If the turn stops with `aborted` or `error`, all pending tool cards are retro-fitted with a single red error line and the transcript suggests `/bug`.
- **area:** transcript / errors
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3514-3525` (cards updated, `pendingTools` cleared, `maybeSuggestBugReport`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3499-3511` (the abort message, including `Aborted after N retry attempt(s)`)
- **how to observe it:** start a long tool call and press Escape.
- **notes:** the `/bug` hint is suppressed for **retryable** errors and for any message whose text contains abort/cancel (`interactive-mode.ts:2158-2162`) so it does not nag, and it is shown **at most once per session** (`interactive-mode.ts:546`, `2145-2147`).

### P-052 — Cache-miss notices explain re-billed tokens
- **behaviour:** After a completed assistant message, if it paid for a significant cache miss the transcript gets a warning line: `Cache miss: 18.2K tokens re-billed (~$0.42)`, or a variant naming the cause.
- **area:** transcript / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4100-4126` (`maybeShowCacheMissNotice` / `addCacheMissNotice`; thresholds at 4114, labels at 4118-4123)
- **how to observe it:** idle for longer than the cache TTL, then send a prompt with `showCacheMissNotices` enabled.
- **notes:** three thresholds a porter will guess wrong: the notice needs **both** `missedTokens ≥ 20 000` **or** `missedCost ≥ 0.1` (`interactive-mode.ts:4114` — it is an `||`, so a cheap huge miss still shows), cost is only printed at `≥ $0.01` (`4116`), and the feature is off by default (`settings-manager.ts:1050-1052`). Notices are re-derived on replay rather than persisted, so they are injected after the assistant message that paid (`interactive-mode.ts:3920-3924`, `3984-3987`).

### P-053 — Anthropic dropped-thinking notice
- **behaviour:** When Anthropic silently dropped thinking blocks on the last turn, the transcript says `Anthropic dropped N thinking block(s) (details in session)`.
- **area:** transcript / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4070-4098` (`maybeShowThinkingDropNotice`, message at 4093), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4054-4068` (`countDroppedThinkingBlocks`)
- **how to observe it:** with a long Anthropic thinking turn followed by a short one.
- **notes:** it is **delta-based** — it only fires when the dropped count rose relative to the previous assistant message (`interactive-mode.ts:4076-4087`). Singular/plural is handled (`4089`). Shares the `showCacheMissNotices` setting, so it is off by default (`4071`).

### P-054 — Compaction renders a summary card plus a billed-cost line
- **behaviour:** A completed compaction clears and rebuilds the transcript, then inserts a compaction summary card at its chronological position followed by `Compaction: <tokens> tokens billed (~$X.XX)`.
- **area:** transcript / compaction
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3623-3673` (`compaction_end`; rebuild at 3643-3645, summary at 3646-3652, cost at 3653-3659), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4041-4052` (`addCompactionCostNotice`, the `$0.01` threshold at 4046), `packages/coding-agent/test/interactive-mode-compaction.test.ts:10-49` (exact strings) and `:109-198` (ordering)
- **how to observe it:** `/compact`.
- **notes:** the compaction summary is the *newest* entry for model context but is appended **at the bottom** of the transcript visually (`interactive-mode.ts:3644` comment). Cost only prints at `≥ $0.01`. The same notice path labels branch summaries `Branch summary:` (`interactive-mode.ts:4047`). Cached replay puts each cost line immediately after **its own** summary (`interactive-mode-compaction.test.ts:51-107`).

### P-055 — Cached sessions report compaction count on load
- **behaviour:** Opening a session whose history contains compactions prints `Session compacted 1 time` / `Session compacted N times`.
- **area:** sessions / compaction
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4128-4143` (`renderInitialMessages`, the count at 4137-4142)
- **how to observe it:** resume a session that was compacted earlier.
- **notes:** singular/plural is explicit (`interactive-mode.ts:4140`), and it counts *all* entries, not just the branch (`4138`).

### P-056 — Skill invocations render as a collapsible block, separate from the text
- **behaviour:** When a submitted message carries a skill block, the skill is rendered as its own collapsible card and any remaining user text is rendered separately beneath it.
- **area:** transcript / skills
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3857-3876` (`parseSkillBlock` → `SkillInvocationMessageComponent`, then the user message)
- **how to observe it:** run a skill from the `/` menu and read the transcript.
- **notes:** the skill card inherits the global expansion state (`interactive-mode.ts:3864`) — Ctrl+O controls it like every other expandable.

### P-057 — Consecutive status messages collapse onto one line
- **behaviour:** Two status messages emitted back to back with nothing between them update the **same** line instead of appending; anything appended in between breaks the run and the next status starts a new line.
- **area:** transcript / status
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3761-3781` (`showStatus`; the coalescing test at 3766-3771), `packages/coding-agent/test/interactive-mode-status.test.ts:81-119` (both cases)
- **how to observe it:** fire `/reload` on a session with a models.json error and watch the log.
- **notes:** this is a deliberate anti-log-spam mechanism (`interactive-mode.ts:3756-3760`). It works by holding references to the last `Spacer` and `ThemedText` (`interactive-mode.ts:494-496`) — a port that appends naively will produce a scrollback full of one-word status lines.

### P-058 — Errors and warnings are transcript lines with a fixed prefix
- **behaviour:** Errors render as `Error: <message>` in the error colour; warnings as `Warning: <message>` in the warning colour; both are appended to the transcript, not shown in the status row.
- **area:** transcript / errors
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4536-4546` (`showError`, `showWarning`)
- **how to observe it:** `/model nonexistent/none`.
- **notes:** `showError` uses `this.outputPad` for left padding while `showWarning` hardcodes `1` (`interactive-mode.ts:4538` vs `4544`) — an inconsistency in the reference, not an accident worth copying blindly.

### P-059 — Managed-tool download progress is one contiguous group
- **behaviour:** `fd`/`rg` download progress is rendered as a single contiguous block; a warning status is prefixed `Warning: ` and coloured differently.
- **area:** startup / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3742-3753` (`showManagedToolStatus`; the one-time leading spacer at 3743-3746), `packages/coding-agent/test/interactive-mode-status.test.ts:122-144` (exact rendered output)
- **how to observe it:** delete `rg`, restart Pi.
- **notes:** it deliberately resets the `lastStatus*` pointers (`interactive-mode.ts:3750-3751`) so a subsequent `showStatus` does not overwrite a download line.

### P-060 — Extension-supplied entries and messages render in the transcript
- **behaviour:** Custom session entries and custom messages are rendered by renderer functions the extension registered; entries with no renderer, or with no content, are skipped entirely.
- **area:** extensions / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3783-3803` (`addCustomEntryToChat`, skip at 3785-3787 and 3790-3792), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3821-3834` (custom messages), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3794-3800` (inserted **above** a streaming component so it does not appear below live text)
- **how to observe it:** an extension appends a custom entry during a turn.
- **notes:** insertion above the streaming component (`interactive-mode.ts:3794-3800`) is a real ordering rule: a custom entry emitted mid-stream must land *before* the text that has not finished streaming, not after it.

---

## F. Model, auth and trust

### P-061 — Anthropic subscription auth warns once
- **behaviour:** When the active Anthropic credential is a subscription (OAuth, or an API key starting `sk-ant-oat`), Pi warns once that third-party harness usage is billed per token.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:317-322` (`ANTHROPIC_SUBSCRIPTION_AUTH_WARNING`, `isAnthropicSubscriptionAuthKey`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5184-5212` (`maybeWarnAboutAnthropicSubscriptionAuth`, once-guard at 5190-5192), `packages/coding-agent/test/interactive-mode-anthropic-warning.test.ts:19-46` (warns exactly once across two calls)
- **how to observe it:** log in to Anthropic via `/login` and read the first warning.
- **notes:** the warning fires for OAuth **without** touching the credential store (`interactive-mode.ts:5198-5201`; asserted at `interactive-mode-anthropic-warning.test.ts:47-65`), is limited to `provider === "anthropic"` (`5193`), is once per process (`5190`), and can be switched off with `warnings.anthropicExtraUsage: false` (`5187-5189`).

### P-062 — `/login` walks auth-type → provider → dialog
- **behaviour:** `/login` with no argument opens an auth-method selector (`Sign in with an account` / `Sign in with an API key`, plus a Radius entry when available); picking one opens a searchable provider list; the choice then opens a login dialog.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5809-5883` (`showLoginAuthTypeSelector`, labels at 5819-5820), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5885-5928` (`showLoginProviderSelector`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5775-5796` (`handleLoginCommand` shortcut when the ref is unambiguous)
- **how to observe it:** `/login`, then `/login anthropic`.
- **notes:** the provider list is sorted by display name, not id (`interactive-mode.ts:5744`). Cancelling a dialog calls `onBack`, which reopens the selector the login was started from (`interactive-mode.ts:5798`, `5912-5914`) — a port that does not thread `onBack` will dump the user back at the composer.

### P-063 — After login Pi picks a default model, or explains why it cannot
- **behaviour:** On successful authentication with no model previously selected, Pi selects the provider's default model and reports `Logged in to X. Selected <model>. Credentials saved to <path>`; when it cannot, it prints the login succeeded **plus** a specific error explaining what to do.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5990-6085` (`completeProviderAuthentication`; success at 6042-6043, each failure message at 6014-6033)
- **how to observe it:** `/login <provider>` on a fresh session.
- **notes:** four distinct failure texts a porter will miss — llama.cpp needs `/llama` first (`6013-6014`); a provider with no configured default (`6015-6016`); a provider with zero models (`6017-6018`); and a default that is not in the catalog (`6025-6026`). For dynamic catalogs the selection is **deferred** until a 15 s refresh completes (`6000-6005`, `6055-6059`), and a model the user selected meanwhile is never clobbered (`6071-6074`).

### P-064 — Login dialogs render auth URLs, device codes, progress and info
- **behaviour:** The login dialog is a single component that renders four event kinds: an auth URL with instructions, a device code plus `Waiting for authentication...`, an info message with links, and progress text.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6227-6238` (`notifyAuthDialog`, one branch per event type), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6203-6225` (`showAuthPrompt`: `select`, `manual_code`, and free-text prompts)
- **how to observe it:** `/login` → any OAuth provider.
- **notes:** an abort signal on the prompt races the user's answer and resolves to `Login cancelled` (`interactive-mode.ts:6212-6224`), which the caller maps back to `onBack` (`6160-6161`).

### P-065 — `amazon-bedrock` gets an extra credentials hint in its API-key dialog
- **behaviour:** The API-key login dialog for `amazon-bedrock` additionally shows that an AWS profile, IAM keys or role credentials work, and points at `providers.md` in the docs directory.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6129-6135` (`providerId === "amazon-bedrock"` branch)
- **how to observe it:** `/login amazon-bedrock`.
- **notes:** the path is `path.join(getDocsPath(), "providers.md")` (`interactive-mode.ts:6133`) — a docs-location coupling a port will not have.

### P-066 — Radius login offers to wire up an MCP server
- **behaviour:** After logging in to Radius, Pi asks whether to configure the Radius MCP server in the global `mcp.json`, adding it if missing, and reloads after writing.
- **area:** model / auth / extensions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6296-6343` (`offerRadiusMcpServer`; prompt at 6320, name collision handling at 6309-6310, reload at 6334)
- **how to observe it:** `/login radius`.
- **notes:** nothing is asked when a global server for that URL already authenticates with this provider (`interactive-mode.ts:6307`). Writing `auth` **replaces** MCP's own OAuth sign-in — `oauth` is deleted from the config (`6315-6316`). The name falls back to `radius-mcp` if `radius` is taken (`6310`). Pi states plainly that it reads `mcp.json` at session start, hence the forced reload (`6333-6334`).

### P-067 — `/logout` removes only stored credentials and says so
- **behaviour:** `/logout` lists stored credentials and removes the selected one; when none exist Pi prints an explicit message that `/logout` does not touch environment variables or `models.json`.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5930-5988` (`showOAuthSelector("logout")`; the empty-state message at 5944-5947), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5966-5978` (per-type confirmation wording)
- **how to observe it:** `/logout` with no credentials saved.
- **notes:** a partial failure is distinguished from a total one: `Credentials removed for X, but local model state could not be synchronized: …` vs `Logout failed: …` (`interactive-mode.ts:5974-5978`). The provider list is read with a 15 s timeout (`5748`).

### P-068 — Pi has no permission system — by design
- **behaviour:** Pi runs every side-effecting tool with the permissions of the user and process that launched it; there is no built-in allow/deny gate, and the documented remedy is containerisation or an extension-provided sandbox.
- **area:** permissions
- **evidence:** `references/pi-1.0.0/README.md:42` ("Pi does not include a built-in permission system for restricting filesystem, process, network, or credential access"), `references/pi-1.0.0/README.md:44-48` (the three documented patterns: Gondolin extension, plain Docker, OpenShell)
- **how to observe it:** run a `bash` tool that touches the filesystem; nothing asks.
- **notes:** this is the single largest behavioural difference from a harness with a permission engine, and it is **intentional**. The only trust gate in the TUI is *project* trust, which decides whether `.pi` resources and packages load at all (see P-010, P-140) — it does not gate tool calls.

---

## G. Sessions

### P-069 — `/new` starts a fresh session and confirms in the transcript
- **behaviour:** `/new` (and the unbound `app.session.new` action) creates a new session and prints `✓ New session started` in the transcript.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6845-6858` (`handleClearCommand`, confirmation at 6853), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3245-3249` (dispatch)
- **how to observe it:** `/new`.
- **notes:** a cancelled new-session prompt (e.g. trust) returns silently with **no** message (`interactive-mode.ts:6849-6851`). The keybinding has **no default** (`keybindings.ts:146`) — this is slash-command-only unless the user binds it.

### P-070 — `/resume` opens a session picker with rename, delete and cross-directory listing
- **behaviour:** The session picker lists sessions for the current directory, can widen to all directories, supports in-picker rename, and offers a quit action.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5632-5673` (`showSessionSelector`; scoped vs global listing at 5634-5645, rename callback at 5659-5666, current session highlighted at 5669)
- **how to observe it:** `/resume`, then arrow down and press the rename key.
- **notes:** the global listing is only used when the session dir is the **default** one; a custom `--session-dir` forces the scoped list (`interactive-mode.ts:5642-5645`). Renaming is refused for an empty name (`5660-5661`). In-picker bindings are `ctrl+p` path toggle, `ctrl+s` sort toggle, `ctrl+n` named-only filter, `ctrl+r` rename, `ctrl+d` delete, `ctrl+backspace` delete-when-query-empty (`keybindings.md:134-145`).

### P-071 — Resuming a session whose cwd no longer exists asks for a replacement
- **behaviour:** If the saved session's working directory is gone, Pi shows a confirm dialog offering a fallback cwd; declining cancels the resume.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2707-2713` (`promptForMissingSessionCwd`, confirm title at 2708), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5691-5707` (resume path), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6508-6521` (import path, same handling)
- **how to observe it:** start a session, delete the directory, resume it from elsewhere.
- **notes:** the messages differ: a successful override says `Resumed session in current cwd` (`5705`) instead of `Resumed session` (`5688`). The same handling exists for `/import` — a port that only covers resume will break on import.

### P-072 — `/tree` offers summarisation before navigating
- **behaviour:** Selecting a tree node closes the selector and asks `Summarize branch?` with three choices: `No summary`, `Summarize`, `Summarize with custom prompt`; cancelling re-opens the tree on the same node.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5513-5541` (the prompt, the re-open loop at 5522-5526 and the custom-instruction loop at 5530-5536)
- **how to observe it:** `/tree`, select an earlier node, press Escape on the summary prompt.
- **notes:** the whole prompt can be skipped with `branchSummarySkipPrompt` (`interactive-mode.ts:5514`). Choosing the current leaf is a no-op with `Already at this point` (`5500-5504`). After committing, an active response is aborted first (`5543-5547`) and compaction is re-checked *after* the abort settles (`5550-5555`); being busy produces `Wait for the current compaction or tree navigation to finish before navigating the session tree.` and, critically, **does not** disturb the other operation's UI (`interactive-mode-tree-navigation.test.ts:78-100`, regression for #9178).

### P-073 — Branch summary is abortable and reports cancellation
- **behaviour:** While a branch summary is being written Pi shows a branch-summary status row and binds Escape to abort it; aborting prints `Branch summarization cancelled` and re-opens the tree on the same node.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5557-5569` (Escape rebound to `abortBranchSummary`, indicator shown), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5577-5582` (abort → re-open), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5598-5603` (Escape handler restored in `finally`)
- **how to observe it:** `/tree`, choose `Summarize`, press Escape.
- **notes:** the Escape handler is restored in a `finally` (`interactive-mode.ts:5598-5603`) — a rejection mid-navigation must not leave Escape bound to a dead abort.

### P-074 — `/fork` starts a new session from an earlier user message and refills the composer
- **behaviour:** `/fork` lists earlier user messages (newest pre-selected); choosing one forks the session there and puts the text you had typed back into the composer.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5424-5460` (`showUserMessageSelector`; empty-state message at 5427-5430; default selection at 5432), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5446-5447` (editor refilled, `Forked to new session`)
- **how to observe it:** `/fork` with an empty composer.
- **notes:** with no forkable messages it says `No messages to fork from` and opens nothing (`interactive-mode.ts:5427-5430`). It is also reachable as `app.session.fork` and, by default, via **double Escape** — no wait, double-Escape defaults to `tree`; `fork` must be set (see P-026). Both bindings are unbound by default (`keybindings.ts:148`).

### P-075 — `/clone` duplicates the session at the current position
- **behaviour:** `/clone` forks the leaf entry *at* its position into a new session, leaving the composer empty; with nothing to clone it says `Nothing to clone yet`.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5462-5481` (`handleCloneCommand`, `position: "at"` at 5470)
- **how to observe it:** `/clone`.
- **notes:** `/clone` is in `BUILTIN_SLASH_COMMANDS` (`core/slash-commands.ts:35`) and documented (`docs/slash-commands.md:29`), but it is **not** in the docs' "Sessions and context" table order — a porter reconstructing from docs alone will miss it.

### P-076 — `/name` sets and shows the session name, and warns when it is normalised
- **behaviour:** `/name <name>` sets the display name; `/name` alone prints `Session name: <name>` or `Usage: /name <name>`; if the stored name differs from what was typed, Pi says so with both values quoted.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6587-6610` (`handleNameCommand`; the normalisation warning at 6603-6605)
- **how to observe it:** `/name my session!` and watch for the normalised-name warning.
- **notes:** the warning quotes both the input and the stored value with `JSON.stringify` (`interactive-mode.ts:6604`), so whitespace differences are visible. The name also changes the terminal title (P-017).

### P-077 — `/session` prints a full statistics block
- **behaviour:** `/session` prints name, file, id, message counts, a token breakdown with cache hit-rate, cache-warming status and a cost breakdown per model, plus cache re-billing totals.
- **area:** sessions / usage
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6612-6691` (`handleSessionCommand`; the renderer at 6628-6686), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6633` (`File:` shows `In-memory` when the session is not persisted)
- **how to observe it:** `/session`.
- **notes:** six details that are easy to get wrong. (a) `Input` is the **sum** of input + cacheRead + cacheWrite, not raw input (`interactive-mode.ts:6645-6647`). (b) The cached/uncached split only appears when there is cache activity (`6648`). (c) The per-model cost breakdown is **suppressed** when there is a single entry for the currently selected model (`6671`). (d) Cache re-billing prints a dollar figure only at `≥ $0.0001` (`6680`). (e) `Cache Warming` is always shown, even when unavailable, saying `Inactive (cache warming unavailable)` (`6660`). (f) The whole block is built lazily inside a function so it follows theme changes (`interactive-mode.ts:6623`).

### P-078 — `/export` writes HTML, or JSONL when the path ends `.jsonl`
- **behaviour:** `/export` writes an HTML export by default; if the argument ends in `.jsonl` it writes JSONL instead. Both report `Session exported to: <path>`.
- **area:** sessions / files
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6439-6455` (`handleExportCommand`, the `.jsonl` branch at 6443), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6457-6484` (`getPathCommandArgument`, quoted-path handling at 6470-6477)
- **how to observe it:** `/export out.jsonl`, then `/export`.
- **notes:** the argument parser handles **quoted** paths with spaces (`interactive-mode.ts:6470-6477`) but an unterminated quote silently yields `undefined` rather than an error (`6473-6474`) — a port must not crash on `/export "foo`.

### P-079 — `/import` replaces the current session, behind a confirmation
- **behaviour:** `/import <path.jsonl>` asks `Replace current session with <path>?` before doing anything; a missing file reports `Failed to import session: …` rather than falling back to anything.
- **area:** sessions / files
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6486-6528` (`handleImportCommand`; confirm at 6493, usage error at 6489, file-not-found at 6522-6525)
- **how to observe it:** `/import` with no argument, then `/import nope.jsonl`.
- **notes:** this is the one command that **destroys** the current session's context, and it is behind an explicit Yes/No (`interactive-mode.ts:6493`, options `["Yes","No"]` via `showExtensionConfirm` at `2698-2705`). A missing cwd is handled the same way as resume (P-071).

### P-080 — `/share` prefers Radius, falls back to a private GitHub gist
- **behaviour:** `/share` uploads the session to Radius when a Radius token exists; otherwise it renders HTML and shells out to `gh gist create --public=false`, printing both a viewer URL and the gist URL.
- **area:** sessions / export
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6530-6539` (dispatch), `packages/coding-agent/src/modes/interactive/session-share.ts:57-96` (`shareSession`, gist fallback at 71-88), `packages/coding-agent/src/modes/interactive/session-share.ts:175` (`gh gist create --public=false`)
- **how to observe it:** `/share` with no Radius login and no `gh`.
- **notes:** (a) The gist is created with `--public=false` — sharing is private by construction (`session-share.ts:175`). (b) Missing `gh` and not-logged-in `gh` produce **different** errors (`session-share.ts:74` vs `78`). (c) The upload visibility is `organization` on the Radius side (`session-share.ts:120`). (d) Work happens in a temp dir that is removed in a `finally` (`session-share.ts:58`, `89-95`). (e) The loader is abortable and cancellation prints `Share cancelled` (`session-share.ts:112-115`).

### P-081 — `/bug` is a consented, choice-driven report
- **behaviour:** `/bug` runs a multi-step flow: a free-text description with a disclosure of exactly what is included, a transcript include/exclude choice, an optional model-written summary, and finally Upload / Export as Zip / Cancel.
- **area:** sessions / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/bug-report.ts:50-105` (`reportBug`), `packages/coding-agent/src/modes/interactive/bug-report.ts:107-147` (`promptForOptions`, the four prompts at 111, 113, 124, 134), `packages/coding-agent/src/modes/interactive/bug-report.ts:44-47` (`DISCLAIMER`, `TRANSCRIPT_NOTE`)
- **how to observe it:** `/bug`, then read each prompt before answering.
- **notes:** (a) Declining the transcript offers a **model-written summary instead**, and its text says the transcript is sent to the provider with the user's credentials while the summary is the only thing attached (`bug-report.ts:128`). (b) Upload names the destination host (`bug-report.ts:138`). (c) Under `PI_OFFLINE` upload is refused with a pointer to the zip export (`bug-report.ts:56-59`). (d) A failed upload offers `Export as Zip` as a fallback rather than just erroring (`bug-report.ts:93-102`). (e) The report is recorded back into the session as a custom entry and clears the crash log (`bug-report.ts:213-227`).

### P-082 — `/reload` re-reads keybindings, extensions, skills, prompts, themes and context files
- **behaviour:** `/reload` replaces the composer with a bordered "Reloading…" box, reloads the whole resource stack, and reports success or `Reload failed: …`; it refuses while a response is streaming or compaction is running.
- **area:** sessions / extensions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6349-6437` (`handleReloadCommand`; refusals at 6350-6357, the box at 6361-6373, the full reload list at 6401-6412)
- **how to observe it:** edit `keybindings.json`, run `/reload`, press the new binding.
- **notes:** (a) The two refusals have distinct texts: `Wait for the current response to finish before reloading.` and `Wait for compaction to finish before reloading.` (`interactive-mode.ts:6351`, `6355`). (b) The reload box is dismissed even on failure (`interactive-mode.ts:6431-6436`). (c) If the session gained a `.pi` directory and was implicitly trusted, the reload **saves** the trust decision and says so (`interactive-mode.ts:5214-5238`, message at 6426). (d) On failure the *previous* editor is restored, not a fresh one (`interactive-mode.ts:6433`).

### P-083 — `/hotkeys` renders the live key table, including extension shortcuts
- **behaviour:** `/hotkeys` prints a bordered markdown table of every active navigation, editing and application binding, in three sections, followed by an `Extensions` section listing extension-registered shortcuts.
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6728-6843` (`handleHotkeysCommand`; the three tables at 6772-6818; the extensions section at 6820-6834)
- **how to observe it:** bind a custom key, then `/hotkeys`.
- **notes:** the table is built from the **resolved** keybindings, not the defaults, so it always shows what is actually bound (`interactive-mode.ts:6724-6770`). The New line row appends a Windows-only note: `New line (Ctrl+Enter on Windows Terminal)` (`6788`). Extension rows fall back to the extension path when a shortcut has no description (`6830`).

### P-084 — `/changelog` shows the whole changelog, newest first
- **behaviour:** `/changelog` renders every changelog entry inside a bordered `What's New` markdown block.
- **area:** startup / slash commands
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6693-6712` (`handleChangelogCommand`; the `reverse()` at 6700)
- **how to observe it:** `/changelog`.
- **notes:** entries are **reversed** before joining (`interactive-mode.ts:6700`), so the newest is at the top. With an empty changelog it prints `No changelog entries found.` (`6703`).

### P-085 — On exit Pi prints the command that resumes the session
- **behaviour:** Quitting a persisted session prints `To resume this session: pi --session <id>` (with `--session-dir` when non-default) to stdout after the TUI stops.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4244-4247` (the print), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:335-348` (`formatResumeCommand`; guards at 336-340)
- **how to observe it:** `/quit` from a saved session.
- **notes:** the hint is suppressed when stdout is not a TTY, when the session is not persisted, or when the file has disappeared (`interactive-mode.ts:336-340`). Arguments are shell-quoted only when necessary, with `'` doubling (`interactive-mode.ts:328-333`). In fullscreen, `fullscreenExitOutput: "transcript"` first **rebuilds the whole transcript on the main screen** so the scrollback survives the alt-screen exit (`interactive-mode.ts:870-877`).

### P-086 — Shutdown is ordered so nothing can repaint a dying frame
- **behaviour:** On quit Pi disables theme auto-sync, drains up to 1000 ms of pending terminal input (to stop Kitty key-release sequences leaking to the parent shell), stops the TUI, disposes the runtime, then exits.
- **area:** lifecycle
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4233-4249` (the ordered quit path; the drain at 4238-4239), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4211-4231` (the signal path, which disposes **before** touching the terminal — with the reasoning in the comment at 4218-4225)
- **how to observe it:** over a slow SSH connection, press Ctrl+D and check the shell did not receive stray escape sequences.
- **notes:** the two paths are deliberately different and the comment explains why: signal-triggered shutdown must emit extension cleanup *before* terminal restore because extension teardown does not write to the tty (`interactive-mode.ts:4219-4225`).

### P-087 — A dead terminal and an uncaught exception are handled, not left raw
- **behaviour:** Write errors with codes `EIO`, `EPIPE` or `ENOTCONN` trigger an emergency exit with code 129; an uncaught exception restores the terminal (cooked mode, cursor, bracketed-paste/Kitty/modifyOtherKeys off), prints the crash plus any extension frames in the stack, and exits 1.
- **area:** lifecycle
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:290-298` (`DEAD_TERMINAL_ERROR_CODES`, `isDeadTerminalError`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4252-4259` (`emergencyTerminalExit`, exit 129), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4272-4294` (`uncaughtCrash`, exit 1)
- **how to observe it:** kill the SSH connection mid-stream, or throw from an extension's async callback.
- **notes:** without the uncaught handler the terminal is left in raw mode with no cursor and needs `stty sane && reset` (`interactive-mode.ts:4264-4267`). A crash is also **persisted** so the next start points at `/bug` (`interactive-mode.ts:4290-4292`, `2124-2138`); the startup warning names the time and message (`1207-1213`).

### P-088 — tmux extended-keys is checked at startup
- **behaviour:** When running under tmux, Pi queries `extended-keys` and `extended-keys-format` and warns if the configuration will break modified Enter keys.
- **area:** platform / startup
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1268-1313` (`checkTmuxKeyboardSetup`; both warnings at 1305 and 1309), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1167-1172` (invoked asynchronously at startup)
- **how to observe it:** run Pi in tmux with `set -g extended-keys off`.
- **notes:** a 2 s timeout on each `tmux show` and **no warning at all** if tmux cannot be queried (`interactive-mode.ts:1277-1280`, `1301-1302`) — silence here is a deliberate decision, not a failure.

### P-089 — An update notification and a package-update notification are rendered as bordered blocks
- **behaviour:** A newer Pi release renders a bordered `Update Available` block with a link to the changelog; outdated installed packages render a bordered `Package Updates Available` block.
- **area:** lifecycle / startup
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4548-4578` (`showNewVersionNotification`, hyperlinks gated on terminal capability at 4554-4556), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4580-4598` (`showPackageUpdateNotification`)
- **how to observe it:** downgrade the installed Pi, or pin an old extension version, and start.
- **notes:** both checks are skipped entirely under `PI_OFFLINE` (`interactive-mode.ts:1136`, `1251-1253`). The release note is rendered as **markdown in muted colour** (`interactive-mode.ts:4569-4571`), and the changelog URL becomes an OSC 8 hyperlink only when the terminal advertises support (`4554-4556`).

---

---

## H. Themes

### P-090 — The default theme is "system", not "dark"
- **behaviour:** With nothing configured Pi generates a theme from the terminal's own reported colours; there is no fixed default palette.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme.ts:757` (`themeName ?? SYSTEM_THEME_NAME`), `packages/coding-agent/src/modes/interactive/theme/theme-controller.ts:175` (`resolveThemeName` falls back the same way)
- **how to observe it:** change the terminal's 16-colour palette, restart Pi — every UI colour moves.
- **notes:** the system theme has no file on disk (`theme.ts:480`). It is always sorted **first** in every theme list (`theme.ts:495-497`).

### P-091 — Only two theme files ship: `dark` and `light`
- **behaviour:** The built-in theme registry contains exactly `dark.json` and `light.json`; everything else in the picker is user- or package-supplied.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme.ts:445-456` (`getBuiltinThemes` hard-codes both files), `packages/coding-agent/src/modes/interactive/theme/dark.json:3-4` (name `dark`, `appearance: "dark"`), `packages/coding-agent/src/modes/interactive/theme/light.json:3-4`
- **how to observe it:** `/settings` → Theme; the first three entries are `system`, `automatic`, then the rest.
- **notes:** "automatic" is a **menu entry, not a theme** — it is a light/dark pair selector (`components/settings-selector.ts:216`, `:223-230`). Both files use a `vars` indirection layer of `okhsl()` values (`dark.json:5-14`, `:16-26`).

### P-092 — A theme name with exactly one slash means "light theme / dark theme"
- **behaviour:** Setting `theme` to `lightName/darkName` makes Pi pick by the terminal's appearance; the stored setting is a pair, and `getTheme()` returns `undefined` for it.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme.ts:652-667` (`parseAutoThemeSetting` requires **exactly one** slash), `packages/coding-agent/src/modes/interactive/theme/theme.ts:669-680` (`resolveThemeSetting` chooses by appearance), `packages/coding-agent/src/core/settings-manager.ts:856-859`
- **how to observe it:** choose the automatic entry in `/settings`, pick two themes, then flip the terminal between light and dark.
- **notes:** two slashes, an empty side, or a name that itself contains `/` is rejected (`theme.ts:652-667`, `:525-531`). A port that stores a single name loses the automatic mode entirely.

### P-093 — The system theme guarantees 4.5:1 body-text contrast
- **behaviour:** The generated system theme computes lightness from per-token contrast curves so body text keeps at least WCAG 2 AA 4.5:1 against whatever background it lands on; hue and saturation are inherited from the terminal's own ANSI palette.
- **area:** themes / accessibility
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/system-theme.ts:350` (`TEXT_MINIMUM_WCAG_CONTRAST = 4.5`), `packages/coding-agent/src/modes/interactive/theme/system-theme.ts:49-64` (`FAMILIES`, hue + ANSI slot per colour family), `packages/coding-agent/src/modes/interactive/theme/system-theme.ts:510-520` (`limitPanel`), `:620-635` (`withTextContrast`)
- **how to observe it:** set a pastel terminal palette — the generated theme stays pastel, not neon.
- **notes:** this is directly relevant to Epsilon's own WCAG floor. Three escape hatches the reference takes: saturation is re-anchored with chroma capped at the source's so a colour cannot gain saturation as it moves (`system-theme.ts:610-617`); with no background reported it falls back to ANSI indices 0–15 with neutral tokens rendered faint (`:642-654`); and a mid-grey background cannot satisfy every level, so the solver relaxes the minimum **amount that fits** rather than failing (`:540-553`).

### P-094 — Appearance detection prefers the terminal's own reported colours
- **behaviour:** Whether Pi considers itself in a light or dark terminal is decided from the reported foreground direction when that is readable, otherwise from whichever of black/white contrasts better with the reported background, then from the terminal's light/dark report, then from `COLORFGBG`, and finally defaults to dark.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/system-theme.ts:416-431` (`terminalAppearance`), `packages/coding-agent/src/modes/interactive/theme/theme.ts:702-710` (`detectTerminalTheme` wrapper), `packages/coding-agent/src/modes/interactive/theme/theme.ts:689-695` (`COLORFGBG` classified Vim-style)
- **how to observe it:** run Pi in a light terminal with no explicit theme and check that the palette is the light one.
- **notes:** a theme file may also **declare** `appearance`, and when it omits one it is detected from the theme's own colours (`theme.ts:229-236`).

### P-095 — Automatic mode follows the terminal's light/dark notification
- **behaviour:** When the theme is the system theme or a light/dark pair, Pi subscribes to the terminal's colour-scheme notification and re-queries its colours on every switch, repainting live.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme-controller.ts:225-229` (`setTerminalColorSchemeNotifications`), `packages/coding-agent/src/modes/interactive/theme/theme-controller.ts:241-247` (`applyTerminalColorSchemeChange` re-queries), `packages/coding-agent/src/modes/interactive/theme/theme-controller.ts:106`, `:125` (gated on system theme or pair)
- **how to observe it:** in automatic mode, flip the OS colour scheme while Pi is running.
- **notes:** auto-sync is deliberately **off** for a plain named theme — a user who picked `dark` does not want it swapped by the terminal (`theme-controller.ts:106`). It is disabled at shutdown (`interactive-mode.ts:4238`, `7026`).

### P-096 — Custom themes load from three places and are watched live
- **behaviour:** Theme files are read from the user themes directory, the project's `.pi/themes/`, and any path in the `themes` settings array; editing the **active** one repaints the running UI within a debounce, and a temporarily broken file leaves the last good theme on screen.
- **area:** themes
- **evidence:** `packages/coding-agent/src/core/resource-loader.ts:1042` (the two default dirs), `packages/coding-agent/src/config.ts:571-573` (`getCustomThemesDir`), `packages/coding-agent/src/modes/interactive/theme/theme.ts:831-861` (debounced watcher; missing-file early return at `:844-846`)
- **how to observe it:** put a theme JSON in `~/.pi/agent/themes`, open the picker, edit it and save — colours change without a restart.
- **notes:** three limitations a porter will trip over. (a) The **file watcher only watches the custom user directory**, not the project directory or the `themes` array (`theme.ts:821-829`). (b) Built-in `dark`/`light` and the generated `system` theme are **deliberately not watched** (`theme.ts:811-819`). (c) Reload errors are swallowed (`theme.ts:857-859`) — a broken theme is a no-op, not a crash. Duplicate theme names are deduped with first-wins (`core/resource-loader.ts:1174-1198`).

### P-097 — A theme that fails to load silently reverts to the system theme
- **behaviour:** If the configured theme cannot be loaded, Pi keeps rendering — in system colours — and only shows the error when the caller asked for it; the remembered theme name is reset to `system`.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme.ts:764-769` (`initTheme` catch, silent fallback, watcher not started), `packages/coding-agent/src/modes/interactive/theme/theme-controller.ts:182-184` (error shown only when `showError`)
- **how to observe it:** set `theme` in settings to a name that does not exist.
- **notes:** `setTheme` still returns `{ success: false, error }` to the caller (`theme.ts:783-792`), and an extension calling `ctx.ui.setTheme("__missing__")` does **not** persist it (`interactive-mode.ts:2626-2632`; asserted at `interactive-mode-status.test.ts:205-229`).

### P-098 — A theme missing a required token is rejected with the token name
- **behaviour:** Selecting a user theme that omits a required colour produces a message naming the missing tokens rather than a stack trace; unknown tokens are also rejected.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme-json.ts:105-138` (`Missing required color tokens`), `packages/coding-agent/src/modes/interactive/theme/theme-schema.json:43-95` (the required token list), `packages/coding-agent/src/modes/interactive/theme/theme-schema.json:322` (`additionalProperties: false`)
- **how to observe it:** delete `"bashMode"` from a copy of `dark.json` and select it.
- **notes:** schema validation is installed **lazily** because the validator costs ~17 MB; without it only "has a colours map" is checked, so a subtly wrong theme loads in the degraded path (`theme.ts:46-55`, `:533-539`). Colour values accept hex, `oklch()`/`okhsl()`, a 0–255 index, `""` for the terminal default, or a `vars` reference; a circular `vars` reference is an error, not a hang (`theme.ts:135-151`; `theme-schema.json:346-359`).

### P-099 — `""` means "the terminal's own default colour"
- **behaviour:** A theme token set to the empty string renders in the terminal's own default foreground/background; tokens are slot-strict, so a background token is only accepted as a background.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme.ts:290-299` (empty emits SGR 39/49 and records a default slot), `packages/coding-agent/src/modes/interactive/theme/theme.ts:119-126` (slot strictness), `packages/coding-agent/src/modes/interactive/theme/theme.ts:216-219` (`GUESSED_DEFAULT_COLORS` used when the terminal reports none)
- **how to observe it:** write `"text": ""` into a custom theme.
- **notes:** several tokens are **optional with fallbacks** — `scrollbarTrack`←`muted`, `scrollbarThumb`←`text`, `thinkingMax`←`thinkingXhigh`, `searchMatchBg`←`selectedBg`, `searchMatchText`←`text` (`theme.ts:164-179`, `:283`). A port that requires all of them will reject themes Pi accepts.

### P-100 — Theme state is shared through a `globalThis` symbol
- **behaviour:** The active theme lives on `globalThis` under a symbol, so the Node module instance and extensions loaded through jiti share one live theme; using `theme` before initialisation throws.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme.ts:722-733` (`Proxy` reading `Symbol.for("@earendil-works/pi-coding-agent:theme")`), `packages/coding-agent/src/modes/interactive/theme/theme.ts:735-738` (the writer also sets the old `@mariozechner` symbol), `packages/coding-agent/src/modes/interactive/theme/theme.ts:730` (`Theme not initialized`)
- **how to observe it:** change the theme from an extension and watch the main UI repaint.
- **notes:** writing the legacy symbol as well (`theme.ts:735-738`) is a deliberate compatibility affordance for older extensions. The same global-sharing trick is not needed in Rust; the *behaviour* a port must reproduce is that an extension-supplied theme change repaints everything.

### P-101 — The settings selector previews a theme as you move the cursor
- **behaviour:** Moving the highlight in `/settings` → Theme live-previews the candidate theme across the whole UI; cancelling restores the theme that was active before.
- **area:** themes / settings
- **evidence:** `packages/coding-agent/src/modes/interactive/theme/theme-controller.ts:146-153` (`preview`), `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:454-457` (restore on cancel), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4947` (`onThemePreview: (themeName) => this.themeController.preview(themeName)`)
- **how to observe it:** `/settings`, arrow down the Theme list and watch the screen recolour.
- **notes:** confirmation goes through `onThemeChange`, which writes the setting **and** applies it (`interactive-mode.ts:4943-4946`) — preview alone must not persist, or cancel cannot restore.

### P-102 — Capability overrides let a user correct a terminal's self-report
- **behaviour:** `terminal.trueColor`, `terminal.images` and `terminal.hyperlinks` override what Pi detected about the terminal, and the override is applied at construction and on every settings reload.
- **area:** themes / terminal capability
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1278-1286` (`getTerminalCapabilityOverrides`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:599` (`setCapabilityOverrides` in the constructor), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2040` (re-applied in `applyRuntimeSettings`)
- **how to observe it:** set `terminal.trueColor: false` in a terminal that claims truecolor.
- **notes:** only `boolean` values and the literal strings `kitty`/`iterm2` override; `"auto"` and any other string are ignored rather than treated as a request (`settings-manager.ts:1282-1284`). Images can be forced off with `false`, which maps to `images: null` (`:1282`).

---

## I. Settings that change what is on screen

### P-103 — Padding is two independent settings, each with a hard clamp
- **behaviour:** `outputPad` controls the left margin of transcript text and is exactly `0` or `1`; `editorPaddingX` controls the inset inside the composer and is clamped to 0–3.
- **area:** layout / settings
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1488-1490` (`outputPad === 0 ? 0 : 1` — anything else becomes 1), `packages/coding-agent/src/core/settings-manager.ts:1482-1486` (editor clamp), `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:31`, `:114` (default 1, applied to assistant text)
- **how to observe it:** set `outputPad: 0` and `editorPaddingX: 3` and compare the transcript margin with the composer inset.
- **notes:** `outputPad` is **not** a free integer — a config value of `2` silently becomes `1` (`settings-manager.ts:1489`). Changing `outputPad` while streaming patches the live components; while idle it rebuilds the whole chat (`interactive-mode.ts:4991-5011`).

### P-104 — The autocomplete dropdown height is clamped to 3–20 rows
- **behaviour:** The number of autocomplete rows shown at once is configurable but clamped to the range 3–20.
- **area:** composer / settings
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1502-1506` (the clamp), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:639` (passed to the editor), `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:842` (the offered values 3/5/7/10/15/20)
- **how to observe it:** `autocompleteMaxVisible: 20` and open a long `@` file list.
- **notes:** the default is 5 (`settings-manager.ts:1498-1500`), not 10.

### P-105 — Mermaid diagrams can render live, at the end, or not at all
- **behaviour:** Mermaid code fences are drawn as Unicode diagrams either as they stream, only once the block is complete, or never.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1512-1515` (`getMermaidRenderingMode`; only `"off"` and `"final"` are honoured), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:513-516` (the transformer is created with `getMode`), `packages/coding-agent/src/modes/interactive/components/mermaid.ts`
- **how to observe it:** set `markdown.mermaid: "final"` and ask for a diagram.
- **notes:** any out-of-enum value silently falls back to `"streaming"` (`settings-manager.ts:1514`) — the default. Markdown code-block indentation is separately configurable via `markdown.codeBlockIndent`, default two spaces (`settings-manager.ts:1508`; `interactive-mode.ts:1365-1370`).

### P-106 — Inline images can be hidden, resized, auto-downscaled or blocked before sending
- **behaviour:** Four distinct image settings: whether images render inline at all, their on-screen width in cells, whether they are downscaled before being sent to the model, and whether they are sent at all.
- **area:** transcript / files
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1288-1290` (`showImages`, default true), `packages/coding-agent/src/core/settings-manager.ts:1301-1307` (`imageWidthCells`, default 60, floored at 1), `packages/coding-agent/src/core/settings-manager.ts:1403-1405` (`autoResize`, default true), `packages/coding-agent/src/core/settings-manager.ts:1416-1418` (`blockImages`, default false)
- **how to observe it:** set `imageWidthCells: 120`, then `blockImages: true` and send an image.
- **notes:** the two `/settings` rows for rendering only appear when the terminal actually reports image support (`components/settings-selector.ts:469`, `:757-774`), but the two **sending** rows always appear (`:776`, `:785`) — a terminal that cannot show images can still be told to send them.

### P-107 — Global settings merge under project settings; an untrusted project contributes nothing
- **behaviour:** A project's `.pi/settings.json` overrides the user's `~/.pi/agent/settings.json`, key by key, with nested objects merged recursively — unless the project is untrusted, in which case its settings are ignored entirely and writes to it are refused.
- **area:** permissions / settings
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:413` (`deepMergeSettings(global, project)`), `packages/coding-agent/src/core/settings-manager.ts:474-476` (project scope returns `{}` when untrusted), `packages/coding-agent/src/core/settings-manager.ts:662-666` (writes refused when untrusted)
- **how to observe it:** put `editorPaddingX: 3` in a project's `.pi/settings.json`; then revoke trust and restart.
- **notes:** `defaultTools` is the one key that does **not** merge recursively — it is replaced (`settings-manager.ts:250-254`). Settings paths are `~/.pi/agent/settings.json` and `<cwd>/.pi/settings.json` (`settings-manager.ts:300-301`, `:426-427`).

### P-108 — Keybindings live in a separate file, and legacy names migrate on load
- **behaviour:** Keybindings are not settings; they are read from `~/.pi/agent/keybindings.json`, and old short ids (`cycleModelForward`, `expandTools`, …) are rewritten to the current `tui.*` / `app.*` ids automatically.
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/core/keybindings.ts:379-383` (`join(getAgentDir(), "keybindings.json")`), `packages/coding-agent/src/core/keybindings.ts:240-300` (the migration table), `packages/coding-agent/src/core/keybindings.ts:320-340` (`migrateKeybindingsConfig`)
- **how to observe it:** put `{"cycleModelForward": "ctrl+9"}` in the file, `/reload`, and press Ctrl+P.
- **notes:** (a) Values must be a string or an array of strings; anything else is **dropped silently** (`keybindings.ts:306-318`). (b) Malformed JSON is ignored, not fatal (`keybindings.ts:360-369`). (c) An empty array **disables** an action's bindings (`docs/keybindings.md:20-26`). (d) When a migrated key collides with an explicitly-set new key, the explicit one wins (`keybindings.ts:332-335`).

### P-109 — "Windows keybindings" is a mode, not just a platform
- **behaviour:** A single predicate decides whether the Windows-flavoured defaults apply: `win32`, **or** Linux with `WSL_DISTRO_NAME` or `WSL_INTEROP` set.
- **area:** keybindings / platform
- **evidence:** `packages/coding-agent/src/core/keybindings.ts:62-67` (`useWindowsKeybindings`), `packages/coding-agent/src/core/keybindings.ts:73` (evaluated once at module load), `packages/coding-agent/docs/keybindings.md:130` ("WSL uses the normal `ctrl+z` and `fg` behavior")
- **how to observe it:** run Pi under WSL and check that `app.message.followUp` is `ctrl+q`, not `alt+enter`.
- **notes:** the **Windows/WSL set** and the **native-Windows-only set** are different: `app.suspend` has no default on native Windows only (`keybindings.ts:96-99`) and still works under WSL, whereas follow-up/dequeue/paste and undo use the WSL-included predicate (`keybindings.ts:79`, `135`, `139`, `143`). `useWindowsKeybindings()` is exported and takes explicit `platform`/`env` arguments, which is how the keybinding tests exercise it (`keybindings.ts:62-65`).

### P-110 — The full keybinding table Pi ships
- **behaviour:** Pi declares 44 app-level actions plus the `tui.*` set inherited from its TUI package; four of them (`app.session.new`, `app.session.tree`, `app.session.fork`, `app.session.resume`) have **no default key at all**.
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/core/keybindings.ts:75-238` (the `KEYBINDINGS` table), `packages/coding-agent/src/core/keybindings.ts:146-149` (the four unbound session actions), `packages/coding-agent/src/core/keybindings.ts:76` (`...TUI_KEYBINDINGS` spread)
- **how to observe it:** `/hotkeys` — the four session actions do not appear there, because `/hotkeys` only prints actions it has a display string for (`interactive-mode.ts:6755-6770`).
- **notes:** the `tui.*` half (editor cursor, editing, select, fullscreen navigation and search) lives in the TUI package and is **spread in** at `packages/coding-agent/src/core/keybindings.ts:76`; the verbatim table is row **L-000** in section J below, with L-001 to L-003 for the rules that matter.

### P-111 — Every outbound warning is individually switchable
- **behaviour:** Startup and runtime warnings are controlled by named flags under `warnings`, each of which can be disabled without affecting the others.
- **area:** settings / diagnostics
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:90-92` (`WarningSettings.anthropicExtraUsage` default true), `packages/coding-agent/src/core/settings-manager.ts:1524` (the getter), `packages/coding-agent/src/core/settings-manager.ts:716-729` (nested settings merge key by key on write)
- **how to observe it:** set `warnings.anthropicExtraUsage: false` and log in to Anthropic — no warning.
- **notes:** the whole `warnings` object is surfaced in `/settings` and editable there (`interactive-mode.ts:4870`, `5054-5056`), so a port should let the user turn off warnings from the UI, not only from a config file.

### P-112 — Steering and follow-up delivery is one-at-a-time by default
- **behaviour:** Queued steering and follow-up messages are delivered one at a time, each waiting for the agent to become idle, unless the user switches the mode to deliver all at once.
- **area:** composer / message queue
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:830-832` (`steeringMode`, default `"one-at-a-time"`), `packages/coding-agent/src/core/settings-manager.ts:840-842` (`followUpMode`, same default), `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:488-502` (the two rows)
- **how to observe it:** set `followUpMode: "all"`, queue three follow-ups during one turn.
- **notes:** the settings row's description **embeds the platform's actual queue key** rather than hard-coding one (`components/settings-selector.ts:499`) — a concrete instance of the "never hardcode a key" rule Pi's own `AGENTS.md` states.

### P-113 — Cache warming is a global-only setting
- **behaviour:** Prompt-cache warming can be off, run while streaming, or also run between turns; it is deliberately readable only from the **global** settings layer.
- **area:** settings / model
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:1023-1026` (reads `globalSettings` only), `packages/coding-agent/src/core/settings-manager.ts:182` (the setting is global-only in the schema), `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:518-525`
- **how to observe it:** put `cacheWarming` in a project's `.pi/settings.json` — it has no effect.
- **notes:** the reason is cost, and it is stated in the reference (a cache refresh costs money). Any value outside `off`/`streaming`/`idle` silently becomes `"streaming"` (`settings-manager.ts:1025`), which is the default. The warm-cache cost is surfaced in `/session` (`interactive-mode.ts:6658-6665`).

### P-114 — The default tool set is four tools, and modifiers compose across settings layers
- **behaviour:** The startup tool set is `[read, bash, edit, write]`; a settings layer that lists only `+name`/`-name` entries **modifies** the inherited set rather than replacing it.
- **area:** tools / settings
- **evidence:** `packages/coding-agent/src/core/settings-manager.ts:215` (`DEFAULT_TOOL_NAMES`), `packages/coding-agent/src/core/settings-manager.ts:236-247` (`resolveDefaultTools` applies modifiers in list order), `packages/coding-agent/src/core/settings-manager.ts:225-230` (the merge rule)
- **how to observe it:** `defaultTools: ["-bash"]` removes bash; `defaultTools: ["grep"]` replaces the whole set.
- **notes:** an **empty array means "no tools"**, not "inherit" (`settings-manager.ts:238`) — a subtle inversion a porter will get backwards.

---

## J. The `tui.*` keybinding table (verbatim)

### L-000 — TUI keybinding table
- **behaviour:** The editor, input, select and fullscreen-transcript bindings Pi inherits from `@earendil-works/pi-tui` and spreads into the app table.
- **area:** keybindings
- **evidence:** `packages/tui/src/keybindings.ts:71-210` (the whole `TUI_KEYBINDINGS` object, opened at `:71` and closed at `:210`), `packages/coding-agent/src/core/keybindings.ts:76` (`...TUI_KEYBINDINGS` spread into `KEYBINDINGS`)
- **how to observe it:** `/hotkeys` prints the Navigation and Editing tables (`packages/coding-agent/src/modes/interactive/interactive-mode.ts:6772-6796`).
- **notes:** see L-001 to L-003 — they are the parts a porter gets wrong.

| id | default keys | description |
|---|---|---|
| `tui.editor.cursorUp` | `up` | Move cursor up |
| `tui.editor.cursorDown` | `down` | Move cursor down |
| `tui.editor.historyPrevious` | *(none)* | Select previous prompt history entry |
| `tui.editor.historyNext` | *(none)* | Select next prompt history entry |
| `tui.editor.cursorLeft` | `left`, `ctrl+b` | Move cursor left |
| `tui.editor.cursorRight` | `right`, `ctrl+f` | Move cursor right |
| `tui.editor.cursorWordLeft` | `alt+left`, `ctrl+left`, `alt+b` | Move cursor word left |
| `tui.editor.cursorWordRight` | `alt+right`, `ctrl+right`, `alt+f` | Move cursor word right |
| `tui.editor.cursorLineStart` | `home`, `ctrl+home`, `ctrl+a` | Move to line start |
| `tui.editor.cursorLineEnd` | `end`, `ctrl+end`, `ctrl+e` | Move to line end |
| `tui.editor.jumpForward` | `ctrl+]` | Jump forward to character |
| `tui.editor.jumpBackward` | `ctrl+alt+]` | Jump backward to character |
| `tui.editor.pageUp` | `pageUp`, `ctrl+pageUp` | Page up |
| `tui.editor.pageDown` | `pageDown`, `ctrl+pageDown` | Page down |
| `tui.editor.deleteCharBackward` | `backspace` | Delete character backward |
| `tui.editor.deleteCharForward` | `delete`, `ctrl+d` | Delete character forward |
| `tui.editor.deleteWordBackward` | `ctrl+w`, `alt+backspace` | Delete word backward |
| `tui.editor.deleteWordForward` | `alt+d`, `alt+delete` | Delete word forward |
| `tui.editor.deleteToLineStart` | `ctrl+u` | Delete to line start |
| `tui.editor.deleteToLineEnd` | `ctrl+k` | Delete to line end |
| `tui.editor.yank` | `ctrl+y` | Yank |
| `tui.editor.yankPop` | `alt+y` | Yank pop |
| `tui.editor.undo` | `ctrl+-` | Undo |
| `tui.input.newLine` | `shift+enter`, `ctrl+j` | Insert newline |
| `tui.input.submit` | `enter` | Submit input |
| `tui.input.tab` | `tab` | Tab / autocomplete |
| `tui.input.copy` | `ctrl+c` | Copy selection |
| `tui.select.up` | `up` | Move selection up |
| `tui.select.down` | `down` | Move selection down |
| `tui.select.pageUp` | `pageUp` | Selection page up |
| `tui.select.pageDown` | `pageDown` | Selection page down |
| `tui.select.confirm` | `enter` | Confirm selection |
| `tui.select.cancel` | `escape`, `ctrl+c` | Cancel selection |
| `tui.altScreen.pageUp` | `pageUp` | Scroll viewport up one page |
| `tui.altScreen.pageDown` | `pageDown` | Scroll viewport down one page |
| `tui.altScreen.halfPageUp` | *(none)* | Scroll viewport up half a page |
| `tui.altScreen.halfPageDown` | *(none)* | Scroll viewport down half a page |
| `tui.altScreen.lineUp` | *(none)* | Scroll viewport up one line |
| `tui.altScreen.lineDown` | *(none)* | Scroll viewport down one line |
| `tui.altScreen.previousPrompt` | `ctrl+shift+up`, `ctrl+up` | Jump to previous semantic prompt |
| `tui.altScreen.nextPrompt` | `ctrl+shift+down`, `ctrl+down` | Jump to next semantic prompt |
| `tui.altScreen.search` | `ctrl+shift+f` | Search the primary scroll view |
| `tui.altScreen.searchNext` | `enter`, `ctrl+g` | Select the next search match |
| `tui.altScreen.searchPrevious` | `shift+enter`, `ctrl+shift+g` | Select the previous search match |
| `tui.altScreen.searchClose` | `escape` | Close transcript search |
| `tui.altScreen.top` | `home` | Scroll viewport to top |
| `tui.altScreen.bottom` | `end` | Scroll viewport to bottom |

### L-001 — Six of these actions ship unbound
- **behaviour:** `historyPrevious`, `historyNext`, `halfPageUp`, `halfPageDown`, `lineUp` and `lineDown` have handlers but **no default key**; they do nothing until the user binds them.
- **area:** keybindings
- **evidence:** `packages/tui/src/keybindings.ts:74-81` (`historyPrevious`/`historyNext`, `defaultKeys: []`), `packages/tui/src/keybindings.ts:168-183` (the four `altScreen` scroll actions, all `defaultKeys: []`)
- **how to observe it:** press PageUp in fullscreen — it scrolls a whole page, not half; bind `tui.altScreen.lineUp` and it scrolls one line.
- **notes:** Pi does **not** advertise them in `/hotkeys`, because that command only prints ids it has a resolved key for (`packages/coding-agent/src/modes/interactive/interactive-mode.ts:6755-6770`).

### L-002 — Fullscreen bindings deliberately shadow the editor's
- **behaviour:** In fullscreen, `pageUp`, `pageDown`, `home` and `end` scroll the transcript instead of moving the editor cursor.
- **area:** keybindings / layout
- **evidence:** `packages/tui/src/keybindings.ts:159` (the comment: "These intentionally shadow the unmodified editor bindings in fullscreen mode"), `packages/tui/src/keybindings.ts:160-166` and `:208-209` (the shadowing ids), `packages/coding-agent/docs/keybindings.md:100`
- **how to observe it:** switch to fullscreen and press Home — the transcript jumps to the top, the composer cursor does not move.
- **notes:** the same key has a *mode-dependent* meaning. A port with one dispatch table must let the alt-screen table win when the viewport has focus.

### L-003 — A user `[]` disables a binding; conflicts are collected, not rejected
- **behaviour:** User bindings replace defaults per id, an explicit empty list disables an action rather than falling back to its default, and two ids claiming the same key produce a recorded conflict rather than an error.
- **area:** keybindings
- **evidence:** `packages/tui/src/keybindings.ts:265` (empty array disables), `packages/tui/src/keybindings.ts:257-261` (conflicts collected), `packages/coding-agent/docs/keybindings.md:20-26`
- **how to observe it:** put `{"tui.altScreen.pageUp": []}` in `keybindings.json`, `/reload`, press PageUp in fullscreen — nothing happens.
- **notes:** several defaults legitimately collide (`ctrl+d` is both `deleteCharForward` and `app.exit`), so conflict *collection* rather than rejection is load-bearing.

---

## K. Project trust (the only gate Pi has)

### P-115 — `/trust` offers four or five options, including a parent-folder trust
- **behaviour:** The trust dialog lists `Trust`, `Trust parent folder (<path>)`, `Do not trust`, and — when the session-only variants are enabled — `Trust (this session only)` / `Do not trust (this session only)`. It shows the cwd, the saved decision and whether the current session is trusted.
- **area:** permissions
- **evidence:** `packages/coding-agent/src/core/trust-manager.ts:67-97` (`getProjectTrustOptions`; the parent option and its two updates at 73-83), `packages/coding-agent/src/modes/interactive/components/trust-selector.ts:52-70` (the dialog header: cwd, saved decision, current session)
- **how to observe it:** `/trust` in a subdirectory of a project.
- **notes:** trusting the parent is a **two-write** operation: it sets the parent to `true` **and** clears the current directory's own decision to `null`, so the inherited value applies (`trust-manager.ts:77-80`). A `null` value deletes the key rather than storing it (`trust-manager.ts:237-241`). The saved option is pre-selected (`:45-48`) and marked with a ✓ (`:110`).

### P-116 — Trust is inherited from the nearest ancestor with a decision
- **behaviour:** A directory with no saved decision inherits the decision of the nearest ancestor that has one, walking up to the filesystem root.
- **area:** permissions
- **evidence:** `packages/coding-agent/src/core/trust-manager.ts:45-59` (`findNearestTrustEntry`, the walk at 46-58), `packages/coding-agent/src/modes/interactive/components/trust-selector.ts:21-30` (`inherited from <path>` in the label)
- **how to observe it:** trust `/work`, then start Pi in `/work/sub/dir` — it is trusted, and `/trust` says `trusted (inherited from /work)`.
- **notes:** the store is a single `trust.json` under the agent directory, keyed by canonicalised absolute path, and **every** read and write takes a lockfile (`trust-manager.ts:214`, `:221-226`, `:232-245`). It is written sorted with 2-space indentation so it stays diff-stable (`trust-manager.ts:126-136`).

### P-117 — Exactly eight resource kinds make a project "trust-requiring"
- **behaviour:** A project is only asked about when it contains one of eight specific resources under `.pi/` — `settings.json`, `mcp.json`, `extensions`, `skills`, `prompts`, `themes`, `SYSTEM.md`, `APPEND_SYSTEM.md` — or an `.agents/skills` directory in it or an ancestor.
- **area:** permissions
- **evidence:** `packages/coding-agent/src/core/trust-manager.ts:30-39` (`TRUST_REQUIRING_PROJECT_CONFIG_RESOURCES`), `packages/coding-agent/src/core/trust-manager.ts:186-208` (`hasTrustRequiringProjectResources`; the `.agents/skills` ancestor walk at 196-207)
- **how to observe it:** create a directory with only a README and start Pi — no trust prompt.
- **notes:** the user's own `~/.agents/skills` is **explicitly excluded** even when cwd is `$HOME`, because it is a trusted user resource (`trust-manager.ts:183-185`, `:187-188`, `:198`). A port that trusts on "any dotfile directory present" will prompt in directories Pi leaves alone.

### P-118 — Saving a trust decision requires a restart to take effect
- **behaviour:** `/trust` writes the decision and says `Restart pi for this to take effect`; the current session's trust state is shown separately in the dialog and is not changed by the save.
- **area:** permissions
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5240-5263` (`showTrustSelector`; the message at 5253), `packages/coding-agent/src/modes/interactive/components/trust-selector.ts:68` (`Current session: trusted|untrusted`)
- **how to observe it:** `/trust` → `Trust` → read the status line → the session is still untrusted until restart.
- **notes:** there is exactly one in-session exception: if an implicitly-trusted session runs `/reload` **after** gaining a `.pi` directory, Pi saves the trust then and says `saved project trust` (`interactive-mode.ts:5214-5238`, message at 6426).

### P-119 — An extension error is printed with its stack, indented and dimmed
- **behaviour:** When an extension throws, Pi prints `Extension "<path>" error: <message>` in the transcript with the stack trace beneath it, each line indented and dimmed.
- **area:** extensions / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2989-3002` (`showExtensionError`; the stack slice-and-trim at 2995-2999)
- **how to observe it:** an extension that throws during a command.
- **notes:** the first stack line is **skipped** because it duplicates the error message (`interactive-mode.ts:2995`) — a port that prints the raw stack will show the message twice.

### P-120 — First-time setup is a separate, gated startup dialog
- **behaviour:** On a first run Pi may show a two-step setup dialog: pick a theme (system/dark/light, with live preview), then opt in to anonymous analytics; cancelling skips it entirely.
- **area:** startup / themes
- **evidence:** `packages/coding-agent/src/cli/startup-ui.ts:131-148` (`shouldRunFirstTimeSetup`, the four preconditions at 132-147), `packages/coding-agent/src/modes/interactive/components/first-time-setup.ts:64-70` (the theme step), `:71-88` (the analytics step), `packages/coding-agent/src/cli/startup-ui.ts:204-212` (the component wired up)
- **how to observe it:** delete `settings.json` with `PI_EXPERIMENTAL=1` and start Pi.
- **notes:** **all four** conditions must hold and a porter will usually implement only the first: official distribution (not a fork/rebrand), experimental features enabled, the default agent directory (no `PI_AGENT_DIR`), and `settings.json` not existing (`startup-ui.ts:132-147`). The dialog also accepts `j`/`k` as bare navigation keys alongside the configured select bindings (`first-time-setup.ts:130-133`).

---

## 12. Transcript message rendering

*(Rows in this section and those that follow were read by a delegated investigator over the 26 files in `modes/interactive/components/` plus `chat-viewport.ts`, `tui-renderer.ts` and `core/footer-data-provider.ts`. I spot-verified the file/line shape; every citation below resolves in `references/pi-1.0.0`.)*

### P-121 — Assistant prose is markdown with a configurable left pad and no vertical padding
- **behaviour:** An assistant message renders its text as markdown indented by `outputPad`, with no blank line of its own, so a tool card following it is not pushed down by an extra gap.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:114`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3893-3900`
- **how to observe it:** look at the vertical gap between an assistant reply and the first tool card beneath it.
- **notes:** the pad is `0` or `1` only (see P-103). Spacing is decided by the component, not by a layout engine.

### P-122 — A blank line precedes an assistant block only when it has visible content
- **behaviour:** A blank line is inserted above an assistant message only if it has non-whitespace text or thinking; a tool-only assistant message gets no leading gap.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:102`
- **how to observe it:** a turn whose assistant message contains only tool calls — the first card sits flush.
- **notes:** a port that unconditionally adds a spacer will show a double blank line above every tool-only turn.

### P-123 — Consecutive thinking blocks are merged into one
- **behaviour:** Two or more adjacent `thinking` content blocks in one assistant message render as a single block joined by a blank line.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:118` (grouping), `:147` (the join)
- **how to observe it:** a reasoning model that emits two thinking blocks in a row.
- **notes:** merging is per message, not per session; interleaved text splits the groups.

### P-124 — A hidden thinking block becomes a dim italic label
- **behaviour:** When thinking is hidden, each thinking block is replaced by a dim italic placeholder reading `Thinking...` unless an extension has set another label.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:145`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:484` (`defaultHiddenThinkingLabel = "Thinking..."`), `:2334-2345` (`setHiddenThinkingLabel`, applied to every live component)
- **how to observe it:** Ctrl+T on a reply that had thinking.
- **notes:** the default string is `"Thinking..."` with an ASCII ellipsis of three dots (`interactive-mode.ts:484`), and the label is pushed to components **already rendered**, not just future ones (`interactive-mode.ts:2336-2343`).

### P-125 — A thinking block can be toggled individually by left-clicking it
- **behaviour:** Left-clicking a thinking block shows or hides just that block, independently of the global Ctrl+T state.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:163` (click handler), `:143` (the per-run index map)
- **how to observe it:** fullscreen mode, click inside a thinking block.
- **notes:** the per-block state is keyed by a run index, so it survives theme and pad changes only until the map is cleared (`assistant-message.ts:143`).

### P-126 — A blank line follows thinking only when more assistant content follows
- **behaviour:** Thinking is followed by a blank line only if another visible assistant block comes after it.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:138` (the lookahead), `:171` (the separator)
- **how to observe it:** a reply ending in thinking then a tool call — no gap before the card.
- **notes:** the reference is explicit that this avoids "a stray blank before tool cards" (`assistant-message.ts:138`).

### P-127 — A length-truncated reply says so
- **behaviour:** When the model stops because it hit the token limit, the assistant block appends `Response was truncated before completion.` in the error colour.
- **area:** transcript / errors
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:182`
- **how to observe it:** force a very long single reply.
- **notes:** this is distinct from an error: `stopReason === "length"` is neither `aborted` nor `error` (`assistant-message.ts:182` vs `:188`/`:195`).

### P-128 — Abort and error text on the assistant block
- **behaviour:** An aborted reply shows its `errorMessage` if that is set and is not the literal `Request was aborted`, otherwise `Operation aborted`; an errored reply shows `Error: <message>` in the error colour.
- **area:** transcript / errors
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:188` (abort), `:195` (error), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3504-3511` (where the host sets that message)
- **how to observe it:** press Escape mid-stream with no tool call in flight.
- **notes:** the host rewrites the abort message to `Aborted after N retry attempt(s)` when retries were involved, so the component shows that instead (`interactive-mode.ts:3505-3510`).

### P-129 — Abort/error text is suppressed when the message contains tool calls
- **behaviour:** When an assistant message contains tool calls, the assistant block shows **no** abort or error text — the error is carried by the tool cards instead.
- **area:** transcript / errors
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:180`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3518-3524` (the tool cards get the error)
- **how to observe it:** Escape during a tool call, then read the transcript.
- **notes:** without this rule the same error appears twice — once on the message, once on each card. A port that prints both will look broken.

### P-130 — A theme change actually repaints assistant markdown
- **behaviour:** Invalidating an assistant component rebuilds its markdown from the cached message object, so a theme switch recolours text that was rendered under the old palette.
- **area:** themes / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:51` (rebuild from cache on invalidate), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4953-4957` (the chat container is invalidated on a mermaid-mode change)
- **how to observe it:** switch theme in `/settings` while a reply is on screen.
- **notes:** this is the difference between caching a rendered string (stale colours) and caching the *source* (rebuildable). Epsilon's own "a frame is not asserted until you have looked at it" rule applies here directly.

### P-131 — Assistant and thinking markdown use distinct transform message types
- **behaviour:** The markdown transformer chain is told whether a block is `assistant` or `assistant-thinking`; thinking is forced italic and uses a dedicated colour token.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:115` (message type passed), `:156` (thinking italic + `thinkingText` colour)
- **how to observe it:** install an extension markdown transformer that upper-cases `assistant-thinking` blocks.
- **notes:** mermaid rendering is deliberately disabled inside `assistant-thinking` (see P-155).

### P-132 — Assistant and user blocks are wrapped in OSC 133 prompt zones
- **behaviour:** Transcript blocks emit OSC 133 `A`/`B`/`C` sequences so a terminal or shell tool can mark prompt boundaries; an assistant block **containing tool calls** is deliberately left unwrapped.
- **area:** transcript / terminal
- **evidence:** `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:80`, `packages/coding-agent/src/modes/interactive/components/user-message.ts:61`
- **how to observe it:** capture the raw output (`PI_TUI_WRITE_LOG`) and look for `133;`.
- **notes:** a port that wraps everything breaks shell prompt markers. The tool-call exception is the non-obvious half.

### P-133 — User text renders as markdown on a filled background
- **behaviour:** A user message renders its text as markdown tinted with the user-message colour on a user-message background that fills the whole block.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/user-message.ts:48`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3869-3884`
- **how to observe it:** read the background colour of your own message block.
- **notes:** it is **not** wrapped in a `Box` — the markdown already paints the background, and a box would duplicate the fill on every line (`user-message.ts:40`, referencing `packages/tui/src/components/box.ts:101`). This is exactly the class of bug AGENTS.md §5 warns about: correct code, wrong frame.

### P-134 — User markdown preserves list markers and escapes that assistant markdown does not
- **behaviour:** Ordered-list markers and backslash escapes survive in user markdown, where the assistant renderer normalises them away.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/user-message.ts:53`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3878`
- **how to observe it:** paste `1. one` into the composer and send it.
- **notes:** a user must be able to see back exactly what they typed, including things the assistant's renderer would clean up.

### P-135 — The user markdown transform is always told the message is not streaming
- **behaviour:** A user block's markdown transformer always receives `isStreaming === false`.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/user-message.ts:55`, contrasted with `packages/coding-agent/src/modes/interactive/components/assistant-message.ts:115`
- **how to observe it:** an extension transformer that branches on the flag.
- **notes:** user text is never partial, so the flag is constant by design.

---

## M. Tool cards

### P-136 — Every tool block is preceded by a blank line
- **behaviour:** A blank line separates each tool card from what precedes it.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:100`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3965`
- **how to observe it:** a turn making several tool calls in a row.
- **notes:** spacing is per-card, so removing it here is what produces the "cards run together" look.

### P-137 — The tool card's background encodes its state
- **behaviour:** A tool block's background is pending while partial, red when the result is an error, and green once settled successfully.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:307`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3574` (partial update), `:3583` (final update with `isError`)
- **how to observe it:** run a tool, watch the card change colour at start and at completion.
- **notes:** colour is the **only** state channel here. Epsilon's own §8 rule — never rely on colour alone — is stricter than Pi; a port should add a glyph or word.

### P-138 — A tool with no registered definition renders bare, with no box
- **behaviour:** A tool that has a registered definition renders inside a padded bordered box; one that does not renders as bare text with no box and no background at all.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:110` (boxed), `:369` (bare), `:421` (the bare layout: bold tool name, `JSON.stringify(args, null, 2)`, output)
- **how to observe it:** call a tool from an MCP server with no custom renderer.
- **notes:** the "no definition" path dumps pretty-printed JSON args — loud on purpose, so an unrendered tool is obvious.

### P-139 — A tool may opt out of Pi's framing entirely
- **behaviour:** A tool renderer declaring `renderShell: "self"` draws its own lines with no box and no background, after one blank line.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:131` (the flag), `:269` (self render path), `:277-281` (blank line, then tool lines, then images)
- **how to observe it:** use the `bash` tool and compare it with a self-shelled tool.
- **notes:** mouse coordinates are shifted down by one row before dispatching into a self-rendered tool, to match that inserted blank line (`tool-execution.ts:297-299`) — a port that forgets this will make clicks land one line off.

### P-140 — A tool that renders nothing collapses to zero lines
- **behaviour:** If a tool's renderers produce no output, its card disappears entirely rather than showing an empty box.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:412`, `:265`
- **how to observe it:** an extension tool whose renderer returns `[]`.
- **notes:** good default — an empty card is worse than no card. The alternative (rendering a placeholder) is what most ports do.

### P-141 — A throwing tool renderer falls back silently
- **behaviour:** If a tool's `renderCall` or `renderResult` throws, the card silently renders the generic call/result text instead.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:333` (try), `:359` (fallback)
- **how to observe it:** write a renderer that throws and read the card.
- **notes:** **contrast** with custom *entry* renderers, which surface the exception (P-155). The inconsistency is real; do not assume one policy for both.

### P-142 — The generic tool fallback truncates to the first 10 lines
- **behaviour:** A tool with no custom result renderer shows the **first** 10 lines when collapsed, followed by `... (N more lines, KEY to expand)`.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:40` (the limit), `:166` (the hint text)
- **how to observe it:** a tool returning 30 lines of output.
- **notes:** **first** 10, not last 10. Pi uses three different truncation policies in one transcript: first-10 here (P-142), last-20 for `!` bash output (P-150), last-5 for the bash **tool** (P-164). A port that normalises them to one policy will differ from Pi in at least two places.

### P-143 — Clicking a tool card toggles its expansion
- **behaviour:** Left-clicking anywhere on a tool card's call or result line toggles that card.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:175-177`
- **how to observe it:** fullscreen mode, click a collapsed tool card.
- **notes:** per-card clicking is layered **on top of** the single global flag (P-144) — the click sets the card, the key sets everything. A port with only the global key loses this.

### P-144 — Expansion is one global flag across every expandable row
- **behaviour:** Ctrl+O flips a single boolean applied to the header, the loaded-resources sections and every expandable transcript child at once.
- **area:** transcript / display
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4474-4490` (the walk over `loadedResourcesContainer` and `chatContainer`), `packages/coding-agent/test/interactive-mode-status.test.ts:146-169`
- **how to observe it:** Ctrl+O with several collapsed cards on screen.
- **notes:** it does **not** touch the pending-messages area (`interactive-mode.ts:4482`), so a deferred card can be the only un-expanded thing.

### P-145 — Collapsed tool arguments are `key=value`, expanded they are `key: value` lines
- **behaviour:** A collapsed call shows its arguments as space-separated `key=value` pairs truncated to 100 characters; expanded it shows one indented `key: value` per line.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/core/tools/render-utils.ts:78` (collapsed), `:93` (expanded)
- **how to observe it:** call `read` with two arguments, Ctrl+O twice.
- **notes:** the 100-character cap is on the whole joined string, not per pair.

### P-146 — Tool arguments stream in live
- **behaviour:** The card's argument display updates as the model streams them, before the tool has started.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:183` (`updateArgs`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3490`
- **how to observe it:** a tool with a large argument object on a slow connection.
- **notes:** distinct from `markExecutionStarted` (P-148), which flips the card into the pending look; the card is showing args **before** anything runs.

### P-147 — The diff preview is computed only once arguments are complete
- **behaviour:** At `message_end` Pi calls `setArgsComplete()` on every pending tool card, which is what triggers the edit tool's asynchronous diff computation.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:194`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3527-3530`
- **how to observe it:** edit a file and watch the diff appear only after the reply finishes.
- **notes:** computing a diff from partial arguments produces a wrong diff; the flag is the guard.

### P-148 — `markExecutionStarted` flips the card to pending
- **behaviour:** When execution actually begins, the card switches to its pending appearance.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:188`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3566`
- **how to observe it:** a tool with a long queue before it starts.
- **notes:** "card exists" and "card is running" are different states in Pi; a port that conflates them shows every tool as running as soon as it is mentioned.

### P-149 — Images inside a tool result render inline, each after a blank line
- **behaviour:** Image content blocks in a tool result are drawn as terminal images, each preceded by a blank spacer.
- **area:** transcript / files
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:384` (the image loop), `:397` (rendering)
- **how to observe it:** a tool that returns a screenshot.
- **notes:** the width in cells is floored and clamped to at least 1 (`tool-execution.ts:254`), matching the `imageWidthCells` setting.

### P-150 — Non-PNG images are converted to PNG on the kitty protocol
- **behaviour:** On terminals using the kitty protocol, images that are not PNG are converted asynchronously and swapped in, keyed by source data plus mime type so a repeat is not reconverted.
- **area:** files / terminal capability
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:214` (the conversion cache), `:395` (the swap)
- **how to observe it:** return a JPEG from a tool in kitty.
- **notes:** the key includes the **bytes**, not just the path, so the same file at two paths converts twice (`tool-execution.ts:214`).

### P-151 — Without image support, an image block degrades to a text indicator
- **behaviour:** On a terminal with no image support, or with images disabled in settings, an image block renders as a text line giving its mime type and dimensions.
- **area:** files / terminal capability
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tool-execution.ts:389`, `packages/coding-agent/src/core/tools/render-utils.ts:51`
- **how to observe it:** turn `showImages` off and look at the same tool result.
- **notes:** the content is **never silently dropped** — a port that hides images entirely loses information the user had.

---

## N. Bash cards

### P-152 — A `!` command renders as a bordered card headed `$ <command>`
- **behaviour:** Bash output renders in a bordered card whose bold header is the literal command prefixed with `$`.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:51` (the header), `:44` (the `$ ` prefix)
- **how to observe it:** `!echo hi`.
- **notes:** the `$` is literal, not a shell prompt — Pi does not reproduce your actual prompt string.

### P-153 — A `!!` command's border is dimmed
- **behaviour:** A command run with `!!` (excluded from model context) gets a **dimmed** border instead of the bash-mode colour.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:37`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6939` (the flag is passed to the constructor)
- **how to observe it:** `!!echo secret` and compare the border with `!echo hi`.
- **notes:** **known defect in the reference** — `updateDisplay()` rebuilds the header with `theme.fg("bashMode")` unconditionally (`bash-execution.ts:138`), so the dim treatment is lost on the first rebuild even though the constructor honours it. A port should decide which behaviour it wants and say so; do not replicate the bug by accident.

### P-154 — A running command shows a cancellable spinner line
- **behaviour:** While a bash command runs, the card shows `Running... (KEY to cancel)` in muted colour.
- **area:** bash / status
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:59` (the line), `:171` (where it is shown)
- **how to observe it:** `!sleep 30`.
- **notes:** the key text is resolved live, so a rebind changes it (`components/keybinding-hints.ts:35`).

### P-155 — Streamed bash output is normalised before display
- **behaviour:** Output chunks are ANSI-stripped, `\r\n` and `\r` are normalised to `\n`, and an unterminated chunk is joined onto the previous line rather than starting a new one.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:83` (the normalisation), `:89` (the join)
- **how to observe it:** `!printf 'a\rb'`.
- **notes:** without this a progress bar's carriage returns render as a staircase. A port that appends raw chunks will regress here.

### P-156 — Collapsed bash output keeps the last 20 visual lines
- **behaviour:** When collapsed, only the **last** 20 wrapped visual lines of output are shown; the truncation is recomputed when the width changes and cached.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:131` (the keep-last window), `:156` (width-aware recompute)
- **how to observe it:** `!seq 1 500`, then resize the terminal.
- **notes:** last-20 here versus first-10 for the generic tool fallback (P-142) and last-5 for the bash tool (P-164). Three policies, one transcript.

### P-157 — The "more lines" hint names the expansion key
- **behaviour:** When lines are hidden the card prints `... N more lines (KEY to expand)` collapsed and `(KEY to collapse)` expanded.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:177` (collapsed), `:184` (expanded)
- **how to observe it:** `!seq 1 500`.
- **notes:** the key text comes from the live keybindings, so the hint follows a rebind.

### P-158 — Exit status is shown as `(exit N)` or `(cancelled)`
- **behaviour:** A cancelled command shows `(cancelled)` in the warning colour; a failed one shows `(exit N)` in the error colour.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:189` (cancelled), `:192` (exit code)
- **how to observe it:** `!false` versus `!sleep 30` then Escape.
- **notes:** success shows neither.

### P-159 — Truncated output names the file that holds the rest
- **behaviour:** When output was truncated to fit the context but a full copy was saved, the card adds `Output truncated. Full output: <path>` in the warning colour.
- **area:** bash / files
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:196`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6991-6997` (where `fullOutputPath` is passed)
- **how to observe it:** a command producing megabytes of output.
- **notes:** the *user's* work is preserved on disk and pointed at — this is the difference between truncation and data loss, and Epsilon's §6 requires the same.

### P-160 — The card shows at most what the model saw
- **behaviour:** Bash display output is truncated to the same line and byte limits the bash **tool** uses, so the card never displays more than the model was given.
- **area:** bash
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bash-execution.ts:122-124`
- **how to observe it:** compare a `!` card with the equivalent tool result for the same command.
- **notes:** an intentional honesty constraint — the transcript must not overstate what the agent received.

---

## O. Diffs

### P-161 — Diff lines are coloured by kind with line numbers alongside
- **behaviour:** Diff context lines are dim, removed lines red, added lines green, and each line carries its number.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/diff.ts:89` (context), `:129` (removed/added), `:8` and `:124` (line-number parsing and re-emission)
- **how to observe it:** have the edit tool change a file.
- **notes:** line numbers are parsed **out of the diff header** and re-emitted, so the gutter is Pi's, not the diff tool's.

### P-162 — Word-level highlighting applies only to a one-line replacement
- **behaviour:** Intra-line inverse highlighting appears only when the hunk is exactly one removed line against one added line; larger hunks show whole lines.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/diff.ts:115` (the 1-for-1 guard), `:26`
- **how to observe it:** edit one word (highlighted) versus rewrite five lines (not highlighted).
- **notes:** a port that word-diffs every hunk will look busier than Pi on multi-line edits.

### P-163 — Indentation is excluded from the intra-line highlight
- **behaviour:** Leading whitespace is skipped when computing the intra-line highlight, so indentation is never inverted.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/diff.ts:39-41`
- **how to observe it:** change only indentation in a file.
- **notes:** small rule, large visual difference on indentation-only diffs.

### P-164 — Tabs are expanded before diffing
- **behaviour:** Tabs are converted to three spaces before both the diff computation and the rendering.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/diff.ts:17` (expansion), `:120`
- **how to observe it:** diff a tab-indented file.
- **notes:** three spaces, not four or eight — a port using a different width will misalign the gutter against Pi's.

### P-165 — Removed lines are emitted before added lines
- **behaviour:** Within a hunk, removals are always printed before additions even if the source interleaved them.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/diff.ts:127-131`
- **how to observe it:** a hunk whose unified diff alternates.
- **notes:** and lines that do not match the diff grammar (such as `--- a/x` and `+++ b/x` headers) pass through verbatim in the dim context colour (`diff.ts:88-89`).

### P-166 — The edit tool uses the same diff renderer for preview and result
- **behaviour:** The edit tool renders both its preview diff and its final result diff through the same function.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/core/tools/renderers/edit.ts:110` (preview), `:146` (result)
- **how to observe it:** watch an edit card — the preview and the final diff look identical.
- **notes:** a port with two diff paths will produce a visible inconsistency between them.

---

## 16. Markdown, mermaid and truncation

### P-167 — Every markdown block passes through a transformer chain
- **behaviour:** Each rendered markdown block passes through a chain of transformers, each told the message type, whether it is streaming, and the width actually available.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/markdown-transform.ts:9`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2186-2188` (`getMarkdownTransformers`)
- **how to observe it:** install an extension markdown transformer and watch it fire on assistant, user and thinking blocks.
- **notes:** the **available width** is passed, not the terminal width — a transformer must not assume the full screen.

### P-168 — A misbehaving transformer is skipped, never fatal
- **behaviour:** A transformer that returns a non-string is ignored and the previous text kept; one that throws is skipped and the remaining transformers still run.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/markdown-transform.ts:21-22` (non-string), `:24-26` (throw)
- **how to observe it:** a transformer that returns `undefined`, then one that throws.
- **notes:** an extension bug in a renderer must never take the transcript down.

### P-169 — Mermaid fences become Unicode box diagrams
- **behaviour:** A ```` ```mermaid ```` fenced block is replaced by a Unicode box diagram drawn in the terminal.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/mermaid.ts:60` (the substitution), `:75` (the diagram render)
- **how to observe it:** ask for a mermaid flowchart.
- **notes:** detection matches the **first whitespace-separated word** of the fence info string, case-insensitively (`mermaid.ts:14-15`), so ```` ```mermaid title=x ```` works and ```` ```mermaidfoo ```` does not.

### P-170 — A diagram too wide for the terminal is left as a code block
- **behaviour:** If the rendered diagram is wider than the available width, Pi leaves the original code block in place rather than clipping it.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/mermaid.ts:76` (the width check), `:74`
- **how to observe it:** render a wide diagram in an 80-column pane.
- **notes:** degrading to readable source beats a broken diagram. When the renderer produced **warnings**, the block is also kept and a warning line appended, with `(+N more)` for several (`mermaid.ts:77-78`).

### P-171 — Mermaid diagram colours map to theme tokens
- **behaviour:** Each mermaid span class maps to a named theme colour: border, text, edge, edgeLabel, title, or none.
- **area:** themes / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/mermaid.ts:38` (the mapping), `:49`
- **how to observe it:** switch theme and read the diagram.
- **notes:** so a diagram follows the theme rather than carrying its own palette.

### P-172 — Diagram rows are emitted as inline code spans to survive markdown
- **behaviour:** Each diagram row is emitted as an inline code span with a backtick fence sized past the longest backtick run in the content, and a non-breaking space for blank rows.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/mermaid.ts:22` (fence sizing), `:33` (blank-row substitution)
- **how to observe it:** look at the raw markdown of a rendered diagram.
- **notes:** this is a workaround for markdown collapsing whitespace and eating backticks. A port rendering diagrams straight to the terminal lines does not need it — but must not then run them through a markdown renderer that would.

### P-173 — Mermaid never renders inside a thinking block
- **behaviour:** Mermaid rendering is disabled entirely inside `assistant-thinking` blocks.
- **area:** transcript / markdown
- **evidence:** `packages/coding-agent/src/modes/interactive/components/mermaid.ts:63` (the mode check), `:65` (the thinking exclusion)
- **how to observe it:** a model that thinks in mermaid.
- **notes:** and during streaming the diagram is drawn only when the mode is exactly `streaming`; otherwise the raw block shows until the message settles (`mermaid.ts:66`, `:77`).

### P-174 — Collapsed output is limited by wrapped visual lines, not logical lines
- **behaviour:** Truncation counts **wrapped** lines at the current width, so a single very long minified line cannot flood the screen.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/visual-truncate.ts:41` (the visual measure), `:43`
- **how to observe it:** a tool returning one 10 000-character line.
- **notes:** the caller chooses whether to keep the **last** or the **first** visual lines (`visual-truncate.ts:47-48`), and the skipped count is returned so the caller prints its own hint (`:49`, `:83`).

### P-175 — The truncation preview is cached per width
- **behaviour:** The visual-line preview caches its rendered lines per width, because otherwise every transcript result would be re-wrapped on every frame.
- **area:** transcript / performance
- **evidence:** `packages/coding-agent/src/modes/interactive/components/visual-truncate.ts:78-79`
- **how to observe it:** scroll a long transcript; without the cache, frame cost grows.
- **notes:** the `paddingX` passed to it must match its host — 0 inside a `Box`, 1 in a bare container (`visual-truncate.ts:22`; `bash-execution.ts:156`). Getting it wrong shifts every wrapped line by one column.

### P-176 — The hint line is placed on the side that keeps the relevant content
- **behaviour:** The `... (N more lines)` hint is width-truncated with `...` and placed **before** kept-end lines or **after** kept-start lines.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/modes/interactive/components/visual-truncate.ts:84-85`
- **how to observe it:** compare a keep-last and a keep-first card.
- **notes:** the same shared preview is used by MCP tools and codemode renderers (`packages/coding-agent/src/extensions/mcp/tools.ts:299`; `packages/coding-agent/src/extensions/codemode/renderer.ts:85`), so one policy governs all three.

### P-177 — The bash tool prints its own duration and truncation warnings
- **behaviour:** The bash **tool** (as opposed to `!` bash mode) prints an elapsed/duration line and a `[Truncated: showing N of M lines]` or byte-limit warning.
- **area:** transcript / tools
- **evidence:** `packages/coding-agent/src/core/tools/renderers/bash.ts:98` (duration), `:104` (truncation warning), `:18` and `:72` (the 5-line keep-end preview with `... (N earlier lines, KEY to expand)`)
- **how to observe it:** run the `bash` tool versus typing `!ls`.
- **notes:** **earlier** lines here, versus **more** lines in the `!` card's hint (P-157) — the wording differs too, so a port cannot reuse one string.

---

## Q. Extension-supplied transcript content

### P-178 — An extension message with no renderer gets a labelled default block
- **behaviour:** A custom message with no registered renderer renders a bold `[customType]` label, a blank line, and its text as markdown on the custom-message background.
- **area:** extensions / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-message.ts:92`, `:107`
- **how to observe it:** an extension sends a custom message without registering a renderer.
- **notes:** string content and text content blocks are both flattened to text for this path (`custom-message.ts:98`, `:101`).

### P-179 — A registered message renderer replaces the whole block, box and all
- **behaviour:** A message renderer owns the entire block — Pi adds no box or background around it.
- **area:** extensions / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-message.ts:69`, `:79`
- **how to observe it:** compare an extension message with and without a renderer.
- **notes:** a renderer returning nothing, or throwing, falls through to the default boxed rendering (`custom-message.ts:76`, `:82`) — the opposite of what a custom *entry* renderer does (P-182).

### P-180 — Custom messages without `display` are skipped entirely
- **behaviour:** A custom message that does not carry `display` is not rendered at all.
- **area:** extensions / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3822-3823`
- **how to observe it:** an extension that appends a custom message without setting `display`.
- **notes:** the message is still in the session; it is just not drawn. Silent by design — a port should not "fix" this by rendering it.

### P-181 — Custom session entries are renderer-only
- **behaviour:** A custom session entry is added to the transcript only if its renderer produces content; returning nothing means it is not added.
- **area:** extensions / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-entry.ts:54`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3790-3792`
- **how to observe it:** register an entry renderer that returns `[]`.
- **notes:** and the host still owns spacing — a leading spacer is added by the component regardless (`custom-entry.ts:8`, `:59`).

### P-182 — An entry renderer that throws is the one that surfaces the error
- **behaviour:** Unlike every other extension renderer, a **custom entry** renderer that throws produces a boxed `[customType] renderer failed: <message>` in the error colour.
- **area:** extensions / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-entry.ts:47`, `:50`
- **how to observe it:** an entry renderer that throws.
- **notes:** this is an inconsistency with tool renderers (P-141) and message renderers (P-179), which swallow. Worth a deliberate decision in a port rather than a blanket rule.

---

## R. Summary cards

### P-183 — A compaction entry collapses to a one-line token count
- **behaviour:** A compaction renders collapsed as `Compacted from N tokens (KEY to expand)` in a custom-message-coloured box; expanded it becomes a bold `Compacted from N tokens` header plus the summary as markdown.
- **area:** transcript / compaction
- **evidence:** `packages/coding-agent/src/modes/interactive/components/compaction-summary-message.ts:49` (collapsed), `:51`, `:42` (expanded header), `:44`
- **how to observe it:** `/compact`, then Ctrl+O twice.
- **notes:** the count is formatted with `toLocaleString()`, so the separator varies by locale (`compaction-summary-message.ts:36`).

### P-184 — Branch and skill cards have their own collapsed wording
- **behaviour:** A branch entry collapses to `Branch summary (KEY to expand)`; a skill invocation collapses to `[skill] name (KEY to expand)`, with the name on the **same** line.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/branch-summary-message.ts:50`, `:41`, `packages/coding-agent/src/modes/interactive/components/skill-invocation-message.ts:49-50`, `:39`, `:41`
- **how to observe it:** navigate the tree with summarisation, or invoke a skill.
- **notes:** three different collapsed strings for three different collapsible row types, and only compaction shows a count.

### P-185 — All three summary boxes are individually click-toggleable
- **behaviour:** Compaction, branch and skill cards each toggle on a **left** mouse click, in addition to the global expand key.
- **area:** transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/compaction-summary-message.ts:61`, `packages/coding-agent/src/modes/interactive/components/branch-summary-message.ts:60`
- **how to observe it:** fullscreen mode, click a compaction card.
- **notes:** left button only — a right-click does nothing.

---

## S. Status indicators and loaders

### P-186 — Four status kinds, and clearing is kind-scoped
- **behaviour:** There are four status kinds — `working`, `retry`, `compaction`, `branchSummary` — and clearing by kind only clears a matching indicator.
- **area:** status
- **evidence:** `packages/coding-agent/src/modes/interactive/components/status-indicator.ts:7`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2277-2280`
- **how to observe it:** let a compaction end while a retry indicator would be showing.
- **notes:** without the kind check, a late `clearStatusIndicator("compaction")` would wipe a newly-started retry.

### P-187 — The status is drawn inside the editor's top border
- **behaviour:** The working/compaction/retry status is embedded in the composer's top border rather than occupying a row of its own.
- **area:** layout / status
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2257-2275` (`setEditorWorkingStatusIndicator`), `packages/coding-agent/src/modes/interactive/components/custom-editor.ts:41`
- **how to observe it:** send a prompt and watch the border of the input box.
- **notes:** this is why the status costs no height (P-019). It is a per-editor opt-in (`interactive-mode.ts:220-227`).

### P-188 — The border degrades gracefully when the status does not fit
- **behaviour:** The embedded status is drawn after a `── ` prefix with the editor's hidden-line overflow label `↑ N more` centred in the remaining border; if the two together do not fit, the border degrades to the bare spinner glyph, then to status-only, then to a width-fitted spinner.
- **area:** layout / status
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-editor.ts:45` (the prefix), `:57` (the centred overflow label), `:51` and `:71` (the degradation ladder)
- **how to observe it:** narrow the terminal to 20 columns mid-turn.
- **notes:** three degradation steps, in that order. A port with one fallback will either clip or lose the spinner.

### P-189 — The retry indicator counts down in whole seconds
- **behaviour:** A retry shows `Retrying (attempt/max) in Ns... (KEY to cancel)` with a live per-second countdown and a warning-coloured spinner.
- **area:** status / retries
- **evidence:** `packages/coding-agent/src/modes/interactive/components/status-indicator.ts:56`, `:64`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3681-3683`
- **how to observe it:** trigger a retryable provider error.
- **notes:** `CountdownTimer` fires `onTick` **synchronously in its constructor**, then decrements once per second, and calls `onExpire` at zero after disposing (`components/countdown-timer.ts:19`, `:26`) — so a delay of 0 fires immediately rather than after a second.

### P-190 — Compaction wording depends on why it started
- **behaviour:** The compaction indicator reads `Compacting context...` for a manual compaction, `Auto-compacting...` for a threshold trigger, and is prefixed `Context overflow detected,` on overflow.
- **area:** status / compaction
- **evidence:** `packages/coding-agent/src/modes/interactive/components/status-indicator.ts:88`, `:91`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3618`
- **how to observe it:** `/compact` versus letting a long session auto-compact.
- **notes:** three distinct strings for one status kind. The overflow case explains *why* the context filled, which is the whole point of showing it.

### P-191 — A branch summary is shown as its own status
- **behaviour:** While a branch summary is being written, the status reads `Summarizing branch... (KEY to cancel)`.
- **area:** status / sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/status-indicator.ts:109`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3715`
- **how to observe it:** `/tree` → `Summarize`.
- **notes:** it is a **different kind** from compaction (`interactive-mode.ts:3715`), so clearing a compaction indicator will not remove it.

### P-192 — `renderInBorder` depends on the Loader prepending a blank line
- **behaviour:** The status indicator renders its loader at `width + 2` and takes line index 1, because the loader component always prepends a blank line.
- **area:** status / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/components/status-indicator.ts:25`, `packages/tui/src/components/loader.ts:44`
- **how to observe it:** replace the loader and the editor border silently mis-renders.
- **notes:** this is the most fragile coupling in the status path. A port that reimplements the spinner will break the border without any error.

### P-193 — An embedded working indicator takes the editor's border colour
- **behaviour:** When the working status is embedded, its colours are overridden to the editor's border colour so the spinner and the border agree.
- **area:** status / themes
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2298-2311` (`colorFn`), `packages/coding-agent/src/modes/interactive/components/status-indicator.ts:43`
- **how to observe it:** change the thinking level while working and watch the spinner colour follow the border.
- **notes:** this is why the border colour encodes the thinking level at all — it is a second, ambient channel.

### P-194 — Extension loaders are bordered and cancellable
- **behaviour:** A loader shown for an extension operation is wrapped in borders with a `cancel` key hint beneath it; a non-cancellable one omits the hint.
- **area:** extensions / status
- **evidence:** `packages/coding-agent/src/modes/interactive/components/bordered-loader.ts:36`, `:39`, `:25`, `:46`
- **how to observe it:** `/share` while uploading.
- **notes:** the non-cancellable variant still exposes an `AbortSignal` that is never aborted (`bordered-loader.ts:46`) — a port must not promise a cancel key it does not honour.

### P-195 — Dynamic borders re-fit on every resize
- **behaviour:** A dynamic border emits exactly `width` box-drawing dashes with a floor of 1, so it re-fits on every resize.
- **area:** layout
- **evidence:** `packages/coding-agent/src/modes/interactive/components/dynamic-border.ts:23`
- **how to observe it:** resize while a bordered loader or the changelog block is on screen.
- **notes:** it requires an explicit colour function when used from a separately-loaded module instance, because the global theme object is not the same reference there (`dynamic-border.ts:6`).

### P-196 — Themed text rebuilds on invalidate
- **behaviour:** A themed text line rebuilds its string from a callback on every invalidate, so a theme change repaints status lines that a plain text line would leave stale.
- **area:** themes / transcript
- **evidence:** `packages/coding-agent/src/modes/interactive/components/themed-text.ts:26`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3749`
- **how to observe it:** switch theme with a status line on screen.
- **notes:** this is the mechanism behind "don't bake theme colours into stored strings" (`packages/coding-agent/docs/tui.md:100`). A Rust port caching a styled `String` will regress on theme change.

---

## T. Key hints in text

### P-197 — `alt` is displayed as `option` on macOS
- **behaviour:** Every key hint renders `alt` as `option` when running on macOS.
- **area:** keybindings / platform
- **evidence:** `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts:13`
- **how to observe it:** run Pi on macOS and read the startup header or `/hotkeys`.
- **notes:** display-only; the parsed key id is unchanged.

### P-198 — Hints resolve through the live keybindings manager
- **behaviour:** A hint for a keybinding id is resolved at render time, so a rebind changes every hint in the transcript at once.
- **area:** keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts:35`
- **how to observe it:** rebind `app.tools.expand`, `/reload`, read a card's hint.
- **notes:** this is why no hint anywhere in Pi hard-codes a key. An unbound id yields an **empty** hint rather than an error (`keybinding-hints.ts:30`), so a hint row can quietly lose a segment.

---

## U. Footer and status line

### P-199 — The footer is two lines, plus a third for extension status
- **behaviour:** The footer renders two lines — the working directory line and the stats line — and a third line appears whenever any extension has set a status.
- **area:** footer
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:287`, `:297`, `:292`
- **how to observe it:** run an extension that calls `ctx.ui.setStatus`.
- **notes:** extension statuses are sorted alphabetically by key, with newlines, tabs and carriage returns stripped and runs of spaces collapsed (`footer.ts:292`, `:14`).

### P-200 — The cwd is shortened to `~` only inside `$HOME`
- **behaviour:** The footer shows the working directory with the home directory replaced by `~`, but only when the path is inside `$HOME`; outside it the path is absolute.
- **area:** footer
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:33`, `:44`
- **how to observe it:** run Pi from `/tmp` and from `~`.
- **notes:** the git branch is appended in parentheses and the session name after a `•` (`footer.ts:170`, `:176`).

### P-201 — Token totals are shown as `↑in ↓out Rread Wwrite`
- **behaviour:** The stats line shows token totals with arrow glyphs for input and output and letters for cache reads and writes; each is omitted when zero.
- **area:** footer
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:181`, `:184`, `:130` (`CH<x>%` cache-hit rate)
- **how to observe it:** a turn with no cache activity — the `R`/`W` glyphs disappear.
- **notes:** the cache-hit percentage is appended only when there **was** cache traffic (`footer.ts:186`, `:133`). Counts abbreviate in five branches: `1234→1.2k`, `12345→12k` (rounded, not `12.3k`), `1.2M`, `12M` (`footer.ts:25`, `:29`).

### P-202 — Context percentage is colour-coded at two thresholds
- **behaviour:** The context-usage percentage is coloured as an error above 90% and as a warning above 70%; it reads `?` when unknown and gains an ` (auto)` suffix when auto-compaction is on.
- **area:** footer
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:205`, `:208`, `:162`, `:200`
- **how to observe it:** fill the context, then compact and watch it become `?`.
- **notes:** colour is again the only channel — same accessibility gap as the tool cards (P-137). `?` right after compaction is a deliberate unknown, not a zero.

### P-203 — The model name is right-aligned, with routed-model disambiguation
- **behaviour:** The model name sits at the right of the footer; it reads `no-model` when none is set, carries ` • thinking off` or ` • <level>` for reasoning models, and adds ` → <id>` when a routed (virtual) model actually resolved elsewhere.
- **area:** footer / model
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:220`, `:238`, `:244`, `:241`
- **how to observe it:** select a virtual/routed model and compare the footer with the requested id.
- **notes:** the provider is shown in parentheses only when more than one provider is available **and** it still fits; otherwise it is dropped silently (`footer.ts:249`, `:252`). Truncation is ordered — stats first, then the right side, then dropped entirely if there is no room (`footer.ts:225`, `:267`).

### P-204 — The footer truncates styled strings by display width
- **behaviour:** Footer truncation measures **visible** columns and is applied to already-styled strings; dim is applied separately to the stats and to the right side.
- **area:** footer / terminal
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:225`, `:282`, `:284`
- **how to observe it:** narrow the terminal until the model name disappears.
- **notes:** the separate dim application exists because a colour reset inside the stats would clear an outer dim wrapper (`footer.ts:284`). A naive `chars().take(n)` will cut ANSI escape sequences in half — this is the single most common terminal-rendering bug in a Rust port.

### P-205 — Footer stats are recomputed only when something changed
- **behaviour:** Footer statistics are recalculated only when the session, session id, leaf, entry count or context-window model changes — not on every frame.
- **area:** footer / performance
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:109`, `:143`, `:124` (the usage aggregation)
- **how to observe it:** stream a long reply and watch the footer stay static until the entry count moves.
- **notes:** usage totals are summed over `usage` entries, assistant messages, tool results carrying usage, and branch/compaction entries (`footer.ts:124`, `:136`) — missing any of those undercounts.

### P-206 — The git branch is read from `.git/HEAD` without spawning git
- **behaviour:** The current branch is read by parsing `.git/HEAD`, walking up from the cwd; `gitdir:` files (worktrees) and `commondir` are followed. A detached HEAD renders the literal `detached`.
- **area:** footer / git
- **evidence:** `packages/coding-agent/src/core/footer-data-provider.ts:22`, `:242`, `:245`, `:259`
- **how to observe it:** start Pi in a worktree and in a reftable repository.
- **notes:** the `.invalid` sentinel ref (reftable/unborn HEAD) falls back to actually asking git (`footer-data-provider.ts:259`).

### P-207 — Git branch watching has four platform-specific mechanisms
- **behaviour:** The branch watcher watches the **directory** containing HEAD rather than HEAD itself, debounces refreshes at 500 ms, coalesces in-flight refreshes, polls additionally under WSL on `/mnt/<drive>`, adds reftable watchers, and tears down and retries on watcher error.
- **area:** footer / git / platform
- **evidence:** `packages/coding-agent/src/core/footer-data-provider.ts:313` (watches the directory, because git's atomic write replaces the inode), `:101` (500 ms debounce), `:91` (the WSL `fs.watchFile` 1 s poll), `:344` (reftable watchers), `:302` (teardown and retry)
- **how to observe it:** run Pi in a reftable repo under WSL on a Windows drive and switch branches.
- **notes:** every one of these exists because of a specific filesystem behaviour; each is easy to omit and hard to notice until a user's branch display goes stale. The WSL case is because inotify does not cross the 9p boundary (`footer-data-provider.ts:91`).

### P-208 — An `xp` marker flags experimental builds
- **behaviour:** With `PI_EXPERIMENTAL=1`, a bold warning-coloured `xp` marker is appended to the footer stats.
- **area:** footer / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/components/footer.ts:214`, `packages/coding-agent/src/core/experimental.ts:2`
- **how to observe it:** `PI_EXPERIMENTAL=1 pi`.
- **notes:** a small honesty feature: the user is told the build is not a supported configuration, on screen, always.

---

## V. Extension editors and inputs

### P-209 — Escape only interrupts when autocomplete is not open
- **behaviour:** Escape is passed to the app as "interrupt" **only when the autocomplete popup is closed**; with the popup open it cancels the popup instead.
- **area:** composer / keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-editor.ts:104`, `:113`
- **how to observe it:** type `/mod` to open autocomplete, press Escape — the popup closes; press Escape again — the turn aborts.
- **notes:** this is a two-step escape to interrupt. A port that always aborts will kill the turn when the user meant to dismiss a menu.

### P-210 — Ctrl+D falls through to delete-forward when the editor is not empty
- **behaviour:** Ctrl+D exits only when the editor is empty; otherwise it deletes the character forward.
- **area:** composer / keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-editor.ts:119`, `:124`, `packages/tui/src/keybindings.ts:120-123` (`deleteCharForward` includes `ctrl+d`)
- **how to observe it:** put the cursor mid-word and press Ctrl+D.
- **notes:** two different behaviours share one key; the emptiness check lives in the editor, not the app (P-028).

### P-211 — Explicit history bindings beat app actions
- **behaviour:** When `tui.editor.historyPrevious`/`historyNext` are bound, they take precedence over application actions on the same key while the editor has focus.
- **area:** keybindings / composer
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-editor.ts:130`, `:133`, `packages/coding-agent/docs/keybindings.md:67`
- **how to observe it:** bind `historyPrevious` to `ctrl+p` and Ctrl+P stops cycling models.
- **notes:** this is the escape hatch that lets a user reclaim `ctrl+p` without giving up anything else.

### P-212 — Extension shortcuts are consulted before every built-in key
- **behaviour:** Extension-registered shortcuts are checked before any built-in binding; a handler returning `true` consumes the key.
- **area:** extensions / keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/custom-editor.ts:90`, `:24`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2233-2246`
- **how to observe it:** register a shortcut on a key Pi already uses.
- **notes:** conflicts with a built-in are reported as extension diagnostics (P-008), and the handler runs without blocking input (`interactive-mode.ts:2238-2243`).

### P-213 — The extension editor mirrors the main editor's keys and the external-editor flow
- **behaviour:** An extension's multi-line editor renders in a bordered card with a title, optional description and a hint row; Enter submits, Shift+Enter inserts a newline, and Ctrl+G opens the external editor with the same TUI stop/start dance.
- **area:** extensions / composer
- **evidence:** `packages/coding-agent/src/modes/interactive/components/extension-editor.ts:71`, `:92`, `:83`, `:84`, `:59`, `:127`
- **how to observe it:** an extension calling `ctx.ui.editor("Title")`.
- **notes:** it receives the same `getExternalEditorCommand()` resolution as the main composer (`interactive-mode.ts:2790`), so `$EDITOR` behaves identically in both.

### P-214 — The extension input's placeholder is accepted and ignored
- **behaviour:** The extension single-line input takes a `placeholder` argument and **discards** it; it shows its title with a live `(Ns)` countdown and cancels on expiry.
- **area:** extensions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/extension-input.ts:38` (the parameter is `_placeholder`), `:37`, `:64`, `:65`
- **how to observe it:** call `ctx.ui.input("Title", "type here")` — no placeholder appears.
- **notes:** a real defect in the reference, not a design choice. A port should either render the placeholder or document that it does not — but it should not silently accept an argument and do nothing, which is what Epsilon's "nothing a UI advertises may be a no-op" rule forbids.

---

## W. Logo, viewport and renderer details

### P-215 — The logo's brand colours are fixed, not themed
- **behaviour:** The startup logo is drawn with half-block characters whose brand colours (coral, blue, yellow) stay fixed across themes, following only the terminal's colour mode.
- **area:** themes / startup
- **evidence:** `packages/coding-agent/src/modes/interactive/components/pi-logo.ts:19`, `:23`
- **how to observe it:** switch between `dark` and `light` and watch the logo not change.
- **notes:** a deliberate exception to "everything follows the theme". On Apple Terminal, where half blocks misalign, it is replaced by a two-colour `Pi` wordmark plus the version (`pi-logo.ts:32`).

### P-216 — The version rides on the logo's first line
- **behaviour:** The version is appended to the logo's first line and the first key hint to its second; in wordmark form the hints get their own line.
- **area:** startup
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1004`, `:1005`
- **how to observe it:** read the top two lines of the header.
- **notes:** a layout detail that changes how many rows the header occupies — relevant at 80×24.

### P-217 — Scrolled away from the bottom, a pill offers to jump back
- **behaviour:** In fullscreen, once the user has scrolled away from the latest message, a pill appears reading `↓ Jump to latest message <KEY>`.
- **area:** layout / fullscreen
- **evidence:** `packages/coding-agent/src/modes/interactive/tui-renderer.ts:30`, `:32`, `packages/coding-agent/test/interactive-tui.test.ts:82-109` (asserted on the rendered viewport)
- **how to observe it:** fullscreen, scroll up, read the bottom of the transcript.
- **notes:** the key text is resolved live, so a rebind changes the pill (`tui-renderer.ts:31`). It disappears when the transcript is at the end.

### P-218 — Fullscreen styles search matches and the current match differently
- **behaviour:** Search matches are underlined; the current match is bold and inverse; the search navigation button is underlined on hover.
- **area:** fullscreen / themes
- **evidence:** `packages/coding-agent/src/modes/interactive/tui-renderer.ts:26`, `:27`, `:29`
- **how to observe it:** `ctrl+shift+f` in fullscreen and search.
- **notes:** `ctrl+f` instead of `ctrl+shift+f` on Windows/WSL (`core/keybindings.ts:89-92`).

### P-219 — A copy-selection failure returns the message rather than throwing
- **behaviour:** The fullscreen renderer's copy handler returns a success flag or the error **message string** on failure, rather than throwing.
- **area:** fullscreen / clipboard
- **evidence:** `packages/coding-agent/src/modes/interactive/tui-renderer.ts:39`, `:44`
- **how to observe it:** select text in a terminal with no clipboard tool and press the copy key.
- **notes:** the failure is surfaced to the user as text. A port that propagates the error to a panic will crash the TUI on a missing clipboard helper.

### P-220 — A stable proxy lets components survive a renderer swap
- **behaviour:** Components hold a `TUI` proxy that forwards to whichever renderer is currently active, so the renderer can be replaced between regular and fullscreen at runtime without re-creating components.
- **area:** layout / architecture
- **evidence:** `packages/coding-agent/src/modes/interactive/tui-renderer.ts:53`, `:61`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:620`
- **how to observe it:** `/settings` → change `tui-mode` while an extension widget is mounted.
- **notes:** the proxy re-resolves the method when the underlying renderer changes (`tui-renderer.ts:61-71`) — a real hazard if a port caches a method reference across the swap.

---

## X. Easter eggs (one row each; a porter should decide, not inherit)

### P-221 — `/arminsayshi` plays an animated ASCII portrait
- **behaviour:** Typing `/arminsayshi` appends a 31×36 half-block animated portrait that reveals itself with one of seven random effects (`typewriter`, `scanline`, `rain`, `fade`, `crt`, `glitch`, `dissolve`) and ends with the line `ARMIN SAYS HI`.
- **area:** easter egg
- **evidence:** `packages/coding-agent/src/modes/interactive/components/armin.ts:2`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3266-3269`
- **how to observe it:** type `/arminsayshi`; each run picks a different effect.
- **notes:** hidden from autocomplete (P-042) but fully functional.

### P-222 — Selecting a particular model fires an easter egg automatically
- **behaviour:** Selecting an `opencode` provider model whose id contains `kimi-k2.5` automatically appends a scanline-revealed 32×32 RGB portrait with `Free Kimi K2.5 via OpenCode Zen`, `"Powered by daxnuts"` and `— @thdxr`, followed by `Try OpenCode` and a URL.
- **area:** easter egg
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6911-6915` (the trigger), `packages/coding-agent/src/modes/interactive/components/daxnuts.ts:136`
- **how to observe it:** `/model opencode/<anything containing kimi-k2.5>` — no command needed.
- **notes:** fires on **model selection**, from three call sites: `/model <id>` (`:5130`), the model selector (`:5276`) and post-login (`:6045`). Runs about 2 s at 80 ms per tick.

### P-223 — Clicking the header logo plays a full-screen animation
- **behaviour:** In fullscreen, clicking the header logo plays a full-screen overlay where the screen dissolves into braille dust and the logo flies to centre, grows and spins; any key or click plays the exit in reverse.
- **area:** easter egg
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:1058`, `packages/coding-agent/src/modes/interactive/components/pi-logo-animation.ts:778`
- **how to observe it:** fullscreen mode, click the logo.
- **notes:** the module is loaded lazily on first click (`components/pi-logo-animation.lazy.ts:10`) and the animation aborts if an overlay is already showing — so it cannot be stacked.

### P-224 — `/dementedelves` prints an announcement card
- **behaviour:** `/dementedelves` appends an accent-bordered card reading `pi has joined Earendil` with a blog URL, plus a bundled PNG rendered inline at 56 cells where images are supported.
- **area:** easter egg
- **evidence:** `packages/coding-agent/src/modes/interactive/components/earendil-announcement.ts:32`, `:38`, `:13`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3271-3275`
- **how to observe it:** `/dementedelves` in a terminal without image support.
- **notes:** the image is **silently omitted** when the asset cannot be read; the read is attempted once and the result cached (`earendil-announcement.ts:13`).

---

## 25. Where Pi's TUI is **not** a TUI feature

*(Rows here use the `Y-###` series.)*

Everything below ships in the same `packages/coding-agent` as the interactive mode but is **headless or programmatic**, and is therefore **out of scope** for a terminal-UI port. It is listed so a porter does not mistake it for a missing screen.

### Y-001 — Print mode (`-p` / `--print`) is a single-shot headless run
- **what it is:** Send prompts, print the result, exit. No TUI, no renderer, no editor.
- **evidence:** `packages/coding-agent/src/modes/print-mode.ts:1-6` (the file's own header: "Print mode (single-shot): Send prompts, output result, exit"), `packages/coding-agent/src/cli/args.ts:167` (`--print` / `-p`)
- **why it is out of scope:** it has no screen, no keybindings and no components. Its `text` vs `json` output modes are stdout contracts (`print-mode.ts:9`, `:21-30`).

### Y-002 — JSON event streaming is a stdout protocol, not a view
- **what it is:** A reshaped `AgentSessionEvent` stream written to stdout for `--mode json`.
- **evidence:** `packages/coding-agent/src/modes/json-event.ts:20` (`JsonAgentSessionEvent`), `packages/coding-agent/src/modes/print-mode.ts:13` (`toJsonEvent`)
- **why it is out of scope:** the shape exists because a partial `toolcall_start` cannot be serialised with its `partial` field (`json-event.ts:6-9`). This is a wire contract, not a rendering concern.

### Y-003 — RPC mode is a stdio protocol with its own client
- **what it is:** `--mode rpc` drives the agent over newline-delimited JSON with a request/response command set and an event stream.
- **evidence:** `packages/coding-agent/src/rpc-entry.ts:13` (forces `--mode rpc`), `packages/coding-agent/src/modes/rpc/rpc-types.ts` (the command/response types), `packages/coding-agent/src/modes/index.ts:8-9` (`runRpcMode`, `RpcClient` exports)
- **why it is out of scope:** it is Pi's embedding surface for another process. A TUI port needs none of it.

### Y-004 — The SDK surface is programmatic, not interactive
- **what it is:** `packages/coding-agent/src/core/sdk.ts` plus the `index.ts` public exports let a program construct sessions, prompts and tools directly.
- **evidence:** `packages/coding-agent/src/index.ts` (the public surface), `packages/coding-agent/src/core/sdk.ts:420` (where `thinkingBudgets` is forwarded — enforcement itself lives in `packages/agent`, which was **not** read for this inventory)
- **why it is out of scope:** it has no frame. Anything an SDK caller can do, the TUI reaches through `AgentSession` instead (`interactive-mode.ts:584-596`).

### Y-005 — The CLI flag surface is command-line, not a screen
- **what it is:** argument parsing, `--theme`/`--no-themes`, `--model`, `--models`, `--session`, `--export`, `--mode`.
- **evidence:** `packages/coding-agent/src/cli/args.ts:11` (`type Mode = "text" | "json" | "rpc"`), `:95-105` (`--mode` validation), `:187-202` (theme flags), `:289-396` (the help text)
- **why it is out of scope:** these configure the session the TUI then draws. Only the flags with a **visible** effect in the running TUI are inventoried above (via the settings they write).

### Y-006 — The experimental client/server/coordinator is a distributed runtime
- **what it is:** `src/experimental/` contains a session coordinator, session workers, a relay and a remote catalogue — including `client-tui.ts`, which has its *own* TUI.
- **evidence:** `packages/coding-agent/src/experimental/client-tui.ts` (a separate TUI, 27 kB), `packages/coding-agent/src/experimental/coordinator.ts`, `packages/coding-agent/src/experimental/server.ts`
- **why it is out of scope:** this is a remote-execution architecture, not the interactive mode. **Not inventoried** — see the gaps section.

### Y-007 — HTML export renders the transcript in a browser, not a terminal
- **what it is:** `/export` produces a self-contained HTML page with a bundled `marked`, `highlight.js` and a 80 kB template script.
- **evidence:** `packages/coding-agent/src/core/export-html/index.ts`, `packages/coding-agent/src/core/export-html/vendor/highlight.min.js` (121 kB), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:6447-6449` (the call)
- **why it is out of scope:** the *command* is a TUI feature (P-078); the HTML renderer is not. Note the coupling: the export is themed with the **active terminal theme's name** (`interactive-mode.ts:6448`), so a theme change changes the exported file.

### Y-008 — The package manager CLI and the credential printer are separate entry points
- **what it is:** `package-manager-cli.ts` (35 kB) and `cli/credential-print.ts` run outside the TUI.
- **evidence:** `packages/coding-agent/src/package-manager-cli.ts`, `packages/coding-agent/src/cli/credential-print.ts`
- **why it is out of scope:** **not inventoried.**

### Y-009 — Trust and login *prompts* at startup run on a separate startup TUI
- **what it is:** `cli/startup-ui.ts` builds its **own** `TuiMainScreen` to show the first-time setup dialog, the initial trust prompt and initial credential input, then tears it down before the interactive TUI starts.
- **evidence:** `packages/coding-agent/src/cli/startup-ui.ts:85-95` (`createStartupTui`), `:150-179` (`showStartupSelector`), `:182-219` (`showFirstTimeSetup`), `:221-252` (`showStartupInput`)
- **why it is out of scope (mostly):** it *is* terminal UI, but it is a **different renderer instance with a different lifecycle**. The one behavioural row that belongs to the interactive TUI is P-120 (first-time setup's four gating conditions); everything else here is a startup concern a port may legitimately handle differently.

### Y-010 — The provider/model runtime, auth storage and telemetry are services, not views
- **what it is:** `core/model-runtime.ts`, `core/auth-storage.ts`, `core/provider-composer.ts`, `core/telemetry.ts` and the whole `packages/ai` catalogue.
- **evidence:** `packages/coding-agent/src/core/model-runtime.ts`, `packages/coding-agent/src/core/auth-storage.ts`, `packages/coding-agent/src/core/telemetry.ts:1`
- **why it is out of scope:** the TUI *drives* these (P-046, P-061–P-067) but draws nothing of them directly beyond the status and warning lines already inventoried.

---

## Z. Selectors and overlays

*(Read in full by a delegated investigator: `session-selector.ts`, `session-selector-search.ts`, `model-selector.ts`, `scoped-models-selector.ts`, `thinking-selector.ts`, `settings-selector.ts`, `settings-submenu.ts`, `config-selector.ts`, `tree-selector.ts`, `user-message-selector.ts`, `theme-selector.ts`, `show-images-selector.ts`, `oauth-selector.ts`, `extension-selector.ts`, `trust-selector.ts`, `radius-login-selector.ts`, `login-dialog.ts`, `components/index.ts`. No file was missing. I spot-verified the file/line shape.)*

### The cross-cutting overlay mechanism

#### P-225 — Selectors replace the composer, one at a time
- **behaviour:** Opening a selector clears the editor container, installs the overlay and disposes any previously active one; closing restores the editor and its focus, guarded by a token so a stale callback cannot restore it twice.
- **area:** layout / overlays
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:4797-4820` (`showSelector`), `:4804` (the token guard), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:2676`, `:2688` (the extension-selector path)
- **how to observe it:** `/login` → pick an auth type → pick a provider — the child overlay replaces the parent.
- **notes:** nested flows work because cancelling a child re-opens the parent (`interactive-mode.ts:5916-5922`). There are **two** mechanisms — `showSelector` (token + dispose) and the extension UI path (`interactive-mode.ts:2642-2693`) — and a port must implement both or route everything through one.

#### P-226 — Overlays carry a separate focus target
- **behaviour:** Each overlay is constructed with both a `component` and a distinct `focus` target, which for container-based selectors is the inner list rather than the container.
- **area:** overlays / accessibility
- **evidence:** `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5458` (`focus: selector.getMessageList()`), `:5063` (`focus: selector.getSettingsList()`), `packages/coding-agent/docs/tui.md:48-50`
- **how to observe it:** open `/resume` with an IME active and type — the candidate window appears beside the search box.
- **notes:** a container that does not implement `Focusable` needs this indirection to place the IME cursor. Selectors that own a text field must also **propagate** `focused` to their input; forgetting this is invisible until someone uses an IME (`packages/coding-agent/docs/tui.md:50`).

#### P-227 — Every bordered overlay re-fits its border on resize
- **behaviour:** Bordered overlays draw a dynamic top and bottom border that re-renders `"─" × width` on each frame.
- **area:** overlays / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/components/dynamic-border.ts:23`, `packages/coding-agent/src/modes/interactive/components/model-selector.ts:100`, `:151`, `packages/coding-agent/src/modes/interactive/components/session-selector.ts:746`
- **how to observe it:** resize the terminal with any selector open.
- **notes:** the session selector passes an explicit colour function; most others do not, relying on the global theme (`session-selector.ts:746` vs `model-selector.ts:100`).

#### P-228 — The same physical chord means different things in different overlays
- **behaviour:** `Ctrl+S` sorts sessions in `/resume`, saves models in `/scoped-models` and saves the thinking default in `/thinking`; `Ctrl+D` deletes a session in `/resume` but selects the default tree filter in `/tree`; `Ctrl+P` shows session paths in `/resume`, cycles models in the composer and toggles a provider in `/scoped-models`.
- **area:** keybindings / overlays
- **evidence:** `packages/coding-agent/src/core/keybindings.ts:186` (`app.models.save`), `:210` (`app.tree.filter.default`), `:199` (`app.models.toggleProvider`), `:178` (`app.session.delete`)
- **how to observe it:** press Ctrl+S in each of the three overlays in turn.
- **notes:** all three share one logical id space, so rebinding `app.models.save` changes `/scoped-models` too. A port with a single global action map and no overlay-scoped layer cannot reproduce this — and Epsilon's invariant 4 wants the ids declared once, which is compatible: the *overlay* is the scope, not a second registry.

#### P-229 — Overlay hints come from the live keybinding table
- **behaviour:** Overlay footer hints are generated from the current keybindings, so they reflect the user's `keybindings.json`; `alt` renders as `option` on macOS.
- **area:** keybindings / overlays
- **evidence:** `packages/coding-agent/src/modes/interactive/components/keybinding-hints.ts:34`, `:13`
- **how to observe it:** remap a binding in `keybindings.json`, `/reload`, reopen `/resume` — the hint text changed.
- **notes:** three variants exist and are used inconsistently: `keyHint` (dim key + muted label), `rawKeyHint` (fixed literal) and `keyDisplayText` (capitalised, no label) (`keybinding-hints.ts:30-43`).

---

### `/resume` — the session selector

#### P-230 — Tab switches between current-folder and all sessions
- **behaviour:** Tab toggles the title between `Resume Session (Current Folder)` and `Resume Session (All)`, and a cwd column appears only in the All scope.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:144` (render), `:1031` (`toggleScope`)
- **how to observe it:** `/resume`, press Tab.
- **notes:** Tab is `tui.input.tab`, shared with composer autocomplete (`packages/tui/src/keybindings.ts:145`).

#### P-231 — Three sort modes, cycled with Ctrl+S
- **behaviour:** `Ctrl+S` cycles Threaded → Recent → Fuzzy, and the header label changes to match.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:566`, `:1009`, `:134` (relevance is *labelled* "Fuzzy"), `:714` (threaded is the default)
- **how to observe it:** `/resume`, Ctrl+S three times, read `Sort:` in the header.
- **notes:** the *name* is Fuzzy but the *behaviour* is relevance-ranked by score (`session-selector-search.ts:180`).

#### P-232 — Ctrl+N filters to named sessions only
- **behaviour:** `Ctrl+N` toggles the Name filter between All and Named, hiding sessions whose name is empty after trimming.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:571`, `:1017`, `packages/coding-agent/src/modes/interactive/components/session-selector-search.ts:30`
- **how to observe it:** `/resume`, Ctrl+N.
- **notes:** **a real inconsistency** — this branch matches against the *injected* keybindings manager (`session-selector.ts:571`) while every other branch uses the global `getKeybindings()` (`session-selector.ts:541`). A porter that merges the two sources can break Ctrl+N while everything else works.

#### P-233 — Ctrl+P toggles a session-file-path column
- **behaviour:** `Ctrl+P` toggles a column showing each session's file path, abbreviated to `~/…`.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:577`, `:479`
- **how to observe it:** `/resume`, Ctrl+P — the footer hint flips `path (off)` to `path (on)`.
- **notes:** the same chord is `app.model.cycleForward` in the composer; the overlay scope disambiguates (P-228).

#### P-234 — Deleting a session takes over the whole overlay
- **behaviour:** `Ctrl+D` on a selected session turns the hint line into a red `Delete session? enter confirm · esc cancel` and colours the row error-red; while confirming, **every other key is swallowed**.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:158`, `:402`, `:556` (the swallow)
- **how to observe it:** `/resume`, move the cursor, Ctrl+D, then press Tab — nothing happens.
- **notes:** a full modal takeover inside the overlay, not a separate dialog.

#### P-235 — A gentler delete key that only fires on an empty query
- **behaviour:** `Ctrl+Backspace` starts the same delete confirmation **only when the search box is empty**; otherwise the key is forwarded to the search box as editing.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:600-601`, `:583` (the comment explaining why: some terminals cannot distinguish Ctrl+Backspace from Backspace)
- **how to observe it:** `/resume`, type `foo`, Ctrl+Backspace (nothing deleted); clear the query and press it again (confirmation appears).
- **notes:** this exists purely as a terminal-compatibility escape hatch, alongside `app.session.deleteNoninvasive` (`packages/coding-agent/src/core/keybindings.ts:182-185`).

#### P-236 — The active session cannot be deleted
- **behaviour:** Trying to delete the session you are currently in shows `Cannot delete the currently active session` in error red for 3 seconds and does not enter confirmation.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:407`, `:836`, `:415` (comparison is on `canonicalizePath`, not raw string), `:837` (3000 ms auto-hide)
- **how to observe it:** `/resume` from the live session, cursor on the current row, Ctrl+D.
- **notes:** the comparison is canonicalised, so `./session.jsonl` and the absolute path are correctly treated as the same file.

#### P-237 — Deletion prefers the system trash and falls back to unlink
- **behaviour:** Deleting shells out to `trash` first and falls back to permanent removal; the confirmation differs (`Session moved to trash` vs `Session deleted`), and failure shows `Failed to delete: …` for 3 seconds.
- **area:** sessions / files
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:659`, `:857`, `:862`, `:675` (success claimed if `trash` exits 0 **or** the file is gone), `:658` (a path starting with `-` gets `--` inserted), `:671` (error text truncated to 200 chars)
- **how to observe it:** `/resume`, Ctrl+D, Enter on Linux; on Windows the trash path never exists so it is always a permanent unlink.
- **notes:** **this is the closest thing Pi has to a "never lose a user's work" guarantee**, and it is a *default* rather than a rule. Epsilon's §6 requires confirmation (present) plus a recoverable copy (trash, when available) — a port on Windows would have no recoverable copy at all, which is a finding, not a parity target.

#### P-238 — Renaming replaces the list with a panel
- **behaviour:** `Ctrl+R` replaces the list with a `Rename Session` panel containing a prefilled input; Enter saves, Esc returns to the list.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:590`, `:885`, `:904` (header hidden in rename mode), `:918` (empty name is a silent no-op), `:820` (refused while a load is in flight)
- **how to observe it:** `/resume`, Ctrl+R, type, Enter.
- **notes:** the rename hint only appears when the host sets `showRenameHint` (`session-selector.ts:177`; `interactive-mode.ts:5665`).

#### P-239 — Threaded sessions render as a tree
- **behaviour:** With sort = Threaded and an empty query, sessions are shown as a parent/child tree with `└─ ` / `├─ ` prefixes and `│` gutters, ordered by latest activity in each subtree.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:530` (`buildTreePrefix`), `:209` (`buildSessionTree`), `:380-392` (a query or a non-threaded sort collapses to flat rows), `:224` (orphans become roots)
- **how to observe it:** `/resume` with a forked session present.
- **notes:** parents are keyed on the canonicalised `parentSessionPath`.

#### P-240 — Search covers message bodies, not just titles
- **behaviour:** Typing fuzzy-matches the concatenation of session id, name, **all message text** and cwd.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector-search.ts:26`, `packages/coding-agent/src/modes/interactive/components/session-selector.ts:642`
- **how to observe it:** `/resume`, type a word that only appears in an old reply.
- **notes:** searching message *bodies* is a privacy and index-size consideration a port must reproduce **deliberately**, not by accident.

#### P-241 — Quoted phrases and `re:` switch the search mode
- **behaviour:** A double-quoted segment becomes an exact substring match on whitespace-normalised lowercase text; a query beginning `re:` switches to case-insensitive regular expressions.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector-search.ts:136` (phrase), `:75`, `:46` (regex), `:168`, `:49` (`re:` with an empty pattern yields an error and an empty list)
- **how to observe it:** `/resume`, type `"node cve"`, then `re:^\[tool`.
- **notes:** (a) An **unbalanced quote silently falls back** to plain whitespace tokenisation, with no error shown (`session-selector-search.ts:99`). (b) An invalid regex empties the list rather than reporting an error. (c) Regex mode has no fallback to fuzzy.

#### P-242 — Relevance ordering mixes two scoring scales
- **behaviour:** In Fuzzy sort with a query, results are ordered by match score ascending with ties broken by newest `modified`; in Recent sort the query filters but preserves incoming order.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector-search.ts:180`, `:171`, `:125` and `:144` (regex/phrase scores are `index * 0.1`)
- **how to observe it:** `/resume`, Ctrl+S twice, type a query.
- **notes:** the two score scales are added into one total, so a long fuzzy hit can outrank an early exact hit — a real ordering oddity in the reference, not a designed tie-break.

#### P-243 — The empty state tells you the exact key to press
- **behaviour:** An empty list says one of: `No named sessions in current folder. Press <Ctrl+N> to show all, or Tab to view all.`, `No sessions in current folder. Press Tab to view all.`, or `No sessions found` in All scope.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:429`, `:443`, `:431`
- **how to observe it:** `/resume` in a fresh directory.
- **notes:** the "named" variant appears only when the Named filter is active. The key in the message is rendered from the live table, so it follows a rebind.

#### P-244 — Sessions stream in with a progress count
- **behaviour:** The header shows `Loading n/total` while sessions are read and rows appear incrementally.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:142`, `:956`, `:91` (progress is cleared whenever the flag is re-set), `:955` (stale loads are ignored via an identity check)
- **how to observe it:** `/resume` in a directory with many sessions.
- **notes:** the identity check is what stops a slow first load from overwriting a fast second one.

#### P-245 — A load failure is shown for four seconds, not three
- **behaviour:** A failed load shows `Failed to load sessions: <message>` for 4000 ms with an empty list.
- **area:** sessions / errors
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:1003-1004`
- **how to observe it:** make the session directory unreadable, then `/resume`.
- **notes:** 4000 ms, whereas the delete error uses 3000 ms (`:837`). Inconsistent timeouts inside one component — do not copy the inconsistency without a reason.

#### P-246 — Rows show a message count and a relative age
- **behaviour:** Each row ends with `<messageCount> <age>` where age renders as `now` / `5m` / `3h` / `2d` / `2w` / `3mo` / `1y`; the active session's row is accent-coloured and the selected row gets a background highlight.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/session-selector.ts:35`, `:494`, `:515`
- **how to observe it:** `/resume` with mixed-age sessions.
- **notes:** control bytes `[\x00-\x1f\x7f]` are replaced with spaces in displayed text (`:470`) — the **only** sanitising in this component, and it does not cover bidi characters, which Epsilon's §6 requires.

---

### Model pickers

#### P-247 — Ctrl+L opens the model selector with the argument pre-typed
- **behaviour:** `Ctrl+L` or `/model <text>` opens the model selector with the argument already in its search box; Enter selects the highlighted row.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:117`, `:118`, `:155`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3156`, `:3049`
- **how to observe it:** type `/model gpt` and press Enter.
- **notes:** `initialSearchInput` is applied twice — once via `setValue` (`model-selector.ts:118`) and once via `filterModels` (`:155`).

#### P-248 — Tab switches between all models and the cycling scope
- **behaviour:** When scoped models exist, a `Scope: all | scoped` line is shown and Tab flips between the full catalog and only the models permitted for cycling; with no scoped models the line is replaced by a warning.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:248`, `:364`, `:94` (default is **scoped** whenever any exist), `:110` (the warning text)
- **how to observe it:** `/scoped-models` to enable a subset, then Ctrl+L, then Tab.
- **notes:** the default scope is the *narrow* one, which surprises users who expect "all". The warning names the fix: `Use /login to add providers.`

#### P-249 — Current model first, default second, with badges
- **behaviour:** The list puts the current model first and the configured default second, then sorts by provider, marking the current row `✓` and the default ` · default`.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:231`, `:323`, `:329` (the exact row layout `→ ✓ model-id [provider] · default`)
- **how to observe it:** Ctrl+L and read the top three rows.
- **notes:** the column order is fixed and a porter reproducing "roughly the same" will produce a different-looking list.

#### P-250 — The two model pickers rank differently, on purpose
- **behaviour:** The model selector fuzzy-matches over `provider provider/id provider id name` so a provider name outranks a proxy-style id; the scoped-models selector uses the opposite ordering with the id first.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/model-search.ts:17` (`getModelSelectorSearchText`, provider first), `:7-11` (`getModelSearchText`, id first), `packages/coding-agent/src/modes/interactive/components/model-selector.ts:281`, `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:222`
- **how to observe it:** Ctrl+L and type `anthropic`, versus `/scoped-models` and type `anthropic`.
- **notes:** the reason is stated in the reference: exact `provider/id` queries must outrank proxy ids like `openrouter/openai/gpt-5` (`model-search.ts:13-16`). A port that uses one haystack for both pickers will rank wrong in one of them.

#### P-251 — Typing `def` pins defaults to the top
- **behaviour:** Typing any prefix of `default` moves all default models to the top of the results.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:262`, `:285`
- **how to observe it:** Ctrl+L, type `def`.
- **notes:** `isDefaultSearch` uses `"default".startsWith(normalized)`, so even a single `d` triggers it — typing one letter reshuffles the whole list. Surprising, and cited here so a porter does not rediscover it as a bug.

#### P-252 — Typing jumps the cursor back to the top
- **behaviour:** Entering a query resets the selection to row 0 so the best match is highlighted; clearing the query clamps the index to the restored list.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:301`, contrasted with `packages/coding-agent/src/modes/interactive/components/session-selector.ts:394` (which keeps it)
- **how to observe it:** Ctrl+L, move down three rows, type a letter — the cursor jumps to the top.
- **notes:** the two pickers behave differently on purpose or by accident; either way a port must choose per picker.

#### P-253 — The model selector refreshes catalogs in the background and reports the outcome
- **behaviour:** The selector paints the cached snapshot immediately, then refreshes; success shows `Model catalogs refreshed.` in green, and failures show one of a timeout, `Could not refresh <provider>`, or `Could not refresh N model catalogs (a, b)`.
- **area:** model picker / models
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:154`, `:185`, `:196`, `:216`, `:224` (aborted on cancel and dispose), `:64` (the default `Refreshing model catalogs…`)
- **how to observe it:** Ctrl+L with the network down; read the line under the list.
- **notes:** the timeout is 15 000 ms with an `AbortController`. Showing a **cached** list immediately and refreshing behind it is the behaviour to copy — a port that blocks on the network makes the picker feel broken offline.

#### P-254 — Enter selects, Ctrl+S saves as default, Esc cancels
- **behaviour:** Enter applies the model to the session; `Ctrl+S` applies it **and** writes it to settings; Esc cancels and aborts the refresh.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/model-selector.ts:399`, `:387`, `:394`, `:137` (the Ctrl+S branch exists only when a save callback was supplied, which also controls the hint), `:352`, `:348`
- **how to observe it:** Ctrl+L, highlight a model, Ctrl+S.
- **notes:** the highlighted row's name is printed under the list, replaced by `No matching models` when the filter is empty.

#### P-255 — `/scoped-models` is session-only until you press Ctrl+S
- **behaviour:** The Model Configuration overlay says `Session-only. Ctrl+S to save to settings.` and applies every edit immediately; only Ctrl+S persists.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:137`, `:86-89`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3151`
- **how to observe it:** `/scoped-models`, toggle something and read the subtitle.
- **notes:** the subtitle is the only warning that the change is unsaved — there is no other prompt.

#### P-256 — Enter toggles a model, and "all enabled" materialises into an explicit list
- **behaviour:** Enter enables or disables the highlighted model; the implicit "all enabled" state is materialised into an explicit list on the first toggle, so the display changes from `all enabled` to `n/m enabled`.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:320`, `:30`, `:26` (`normalizeEnabled` collapses back to `null` when everything is re-enabled)
- **how to observe it:** `/scoped-models`, Enter on one row, then re-enable everything.
- **notes:** the `null` sentinel means "no restriction" and is what the host writes as `undefined` to settings (`interactive-mode.ts:5363-5368`). A port that stores an explicit empty list instead will change what Ctrl+P cycles over.

#### P-257 — Bulk actions respect the search filter
- **behaviour:** `Ctrl+A` and `Ctrl+X` operate only on the currently filtered rows when the search box has text, and on everything when it is empty.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:332`, `:342`, `:48`
- **how to observe it:** `/scoped-models`, type `gpt`, Ctrl+A — only the gpt rows change.
- **notes:** clearing with an **empty** filter when nothing was explicitly enabled yields `[]` — i.e. *nothing* enabled (`:48`) — and there is no confirmation before a bulk clear. A destructive action with no confirm and a surprising empty-filter result is worth a deliberate decision in a port.

#### P-258 — Ctrl+P toggles every model of a provider
- **behaviour:** `Ctrl+P` enables every model of the highlighted row's provider, or clears them all if they are already fully enabled.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:352`, `:357`, `:354` (a no-op on unavailable rows)
- **how to observe it:** `/scoped-models`, Ctrl+P on an Anthropic row.
- **notes:** it acts on the provider of the **cursor row**, not of the search query.

#### P-259 — Reordering is disabled until you exclude something
- **behaviour:** `Alt+Up` / `Alt+Down` swap the highlighted model with its neighbour in the enabled list and move the cursor with it — but only once the selection is an explicit list.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:298`, `:309`, `:301` (reorder is disabled while `enabledIds === null`), `:308` (out-of-bounds moves ignored)
- **how to observe it:** `/scoped-models`, try Alt+Up immediately — nothing; disable one model first, then Alt+Up works.
- **notes:** a non-obvious precondition: with "everything enabled" there is no order to change, because the order is the catalog's.

#### P-260 — Unsaved state, unavailable models and the enabled count
- **behaviour:** Any mutation sets a `(unsaved)` marker in warning colour in the footer; a model present in the enabled list but absent from the catalog renders struck through with an `[unavailable]` badge; the footer reports `all enabled` or `n/m enabled · k unavailable`.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:211`, `:369` (save), `:253`, `:272` (strikethrough + badge), `:196`, `:201` (count), `:256` (no `✓` on an unavailable row)
- **how to observe it:** configure `enabledModels` with a stale id, then `/scoped-models`.
- **notes:** ids missing from the catalog are counted as unavailable but are still listed — the user can see and remove them.

#### P-261 — Ctrl+C in the scoped-models overlay is two-stage and hard-coded
- **behaviour:** `Ctrl+C` clears the search box when it has text and only exits the overlay when it is empty; Esc exits immediately.
- **area:** model picker
- **evidence:** `packages/coding-agent/src/modes/interactive/components/scoped-models-selector.ts:377`, `:388`
- **how to observe it:** `/scoped-models`, type text, Ctrl+C — the search clears but the overlay stays.
- **notes:** **these two are matched with `matchesKey`/`Key.ctrl("c")`, not through the keybinding table** — unlike every other component, so they cannot be remapped. Epsilon's invariant 4 would require routing them through the registry instead; a port that reproduces this literally inherits an unremappable key.

---

### `/thinking` — the thinking selector

#### P-262 — The thinking panel is searchable and annotates each level
- **behaviour:** `/thinking` opens a `Thinking Level` panel with a search box that fuzzy-filters over the level name plus its description; each level is annotated (`Light reasoning (~2k tokens)`, `Maximum reasoning`) and the configured default gets a trailing `· default`.
- **area:** model / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/components/thinking-selector.ts:78`, `:119`, `:23`, `:72`
- **how to observe it:** `/thinking`, type `hi`.
- **notes:** the same description table is duplicated in `settings-selector.ts:39` for the per-model submenu — two copies to keep in sync.

#### P-263 — The thinking panel re-selects the surviving value after filtering
- **behaviour:** Filtering rebuilds the list in place and re-selects the previously highlighted value if it survived the filter.
- **area:** model / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/components/thinking-selector.ts:125`, `:123`
- **how to observe it:** `/thinking`, highlight `high`, type `x`, delete it — `high` is still highlighted.
- **notes:** contrast with the settings submenu, which rebuilds with an **empty** preselect and drops the cursor to row 0 (`settings-submenu.ts:115`).

#### P-264 — Ctrl+S in the thinking panel saves the default
- **behaviour:** Enter applies the highlighted level for the session; `Ctrl+S` persists it as the default.
- **area:** model / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/components/thinking-selector.ts:131`, `:114`
- **how to observe it:** `/thinking`, highlight `high`, Ctrl+S.
- **notes:** the Ctrl+S branch exists only when a save callback was supplied; without it the chord falls through to the search box and is **typed as text** (`thinking-selector.ts:131`). A silent typo-into-search-box is exactly the "bound key that does something unintended" failure mode.

#### P-265 — The thinking panel advertises the cycle key
- **behaviour:** The panel prints `<key> cycles thinking levels in-session`, with the key resolved from the live table.
- **area:** model / thinking / keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/thinking-selector.ts:80`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5070`
- **how to observe it:** `/thinking`, read the second line.
- **notes:** navigation keys are routed to the list and **everything else** to the search box (`thinking-selector.ts:137-148`), so any new keybinding must be classified as navigation or it will type into the query.

---

### `/settings`

#### P-266 — `/settings` is a searchable list of roughly thirty settings
- **behaviour:** `/settings` opens a bordered, type-to-search list of settings, each row showing its current value.
- **area:** settings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:868`, `:983`, `:866`, `:987`
- **how to observe it:** `/settings`, type `image`.
- **notes:** the `SettingsList` primitive owns scrolling; the surrounding container only adds the two borders.

#### P-267 — Image rows are capability-gated, the rest are not
- **behaviour:** `Show images` and `Image width` (60/80/120 cells) appear only when the terminal reports image support; every other image setting is always listed.
- **area:** settings / terminal capability
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:469`, `:758`, `:767`
- **how to observe it:** `/settings` on a terminal with images versus one without.
- **notes:** rows are **spliced by index**, so the resulting order is computed rather than declared (`settings-selector.ts:760`, `:777`, `:787`, …). A porter rebuilding this should use an explicit ordered table — the splice approach silently reorders when a row is added.

#### P-268 — The always-present toggles, and their offered ranges
- **behaviour:** Auto-resize images (2000×2000 cap), Block images, Skill commands, Show hardware cursor, Editor padding (0–3), Output padding (0/1), Autocomplete max items (3–20), Clear on shrink and Terminal progress are always listed.
- **area:** settings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:777`, `:838`, `:786`, `:796`, `:806`, `:816`, `:826`, `:836`, `:846`, `:856`
- **how to observe it:** `/settings` and scroll.
- **notes:** each insertion point is computed by `findIndex` on the previous id — the same fragility as P-267.

#### P-269 — Choosing Theme opens a submenu without leaving `/settings`
- **behaviour:** Choosing Theme opens a theme list with `✓` on the current one; selecting `automatic` switches the submenu into split mode in place.
- **area:** themes / settings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:752`, `:313`, `:221`, `:212` (the system theme's description), `:216` (automatic is stored as the sentinel `"/"`)
- **how to observe it:** `/settings` → Theme → arrow to `automatic` + Enter.
- **notes:** the sentinel for "automatic" is the **string `"/"`**, not a named enum value.

#### P-270 — Automatic light/dark has its own four-row submenu
- **behaviour:** In automatic mode the submenu becomes a settings list of `Light theme`, `Dark theme`, `Apply` and `Change mode`; Apply persists the setting as `lightTheme/darkTheme`.
- **area:** themes / settings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:332`, `:446`, `:407`, `:338` (the panel warns `Light/dark detection requires terminal support.`)
- **how to observe it:** `/settings` → Theme → automatic → change Light → Apply.
- **notes:** changing mode back previews the currently active automatic theme (`settings-selector.ts:402`) — a detail that makes the round trip non-destructive.

#### P-271 — Cancelling the theme submenu restores the original theme
- **behaviour:** Moving the theme cursor applies the theme immediately; Esc restores whatever was active when the submenu opened.
- **area:** themes / settings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:325`, `:454`, `:326`, `:355`, `:373`, `:431`, `:434`
- **how to observe it:** `/settings` → Theme, arrow around, Esc — the colours revert.
- **notes:** preview hooks exist on **every** theme list in the submenu, not just the top level — a port that previews only the first list will leave the nested ones committing on selection.

#### P-272 — Per-model thinking is a two-step **looping** submenu
- **behaviour:** `Default thinking level per model` opens step 1 (pick a model) then step 2 (pick a level), and after saving it returns to step 1 rather than closing.
- **area:** settings / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:609`, `:701`, `:37` (step-1 column widths), `:649` (the step-2 title is a function of the context), `packages/coding-agent/src/modes/interactive/components/settings-submenu.ts:226-227` (loop mode)
- **how to observe it:** `/settings` → per-model thinking → choose a model → choose `high` → it stays on step 1.
- **notes:** loop mode rebuilds step 0 with an **empty context**, so nothing is preselected (`settings-submenu.ts:227`). The alternative — close after each save — would make configuring five models five round trips.

#### P-273 — An existing override gains a clear entry
- **behaviour:** If the selected model already has an override, step 2 gains a `(clear override)` entry described `Revert to global default (<level>)`.
- **area:** settings / thinking
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:666`, `:686`, `:188` (the sentinel is `"__clear__"`), `:658` (only reasoning-capable models offer more than `off`)
- **how to observe it:** set an override, reopen the submenu, pick `(clear override)`.
- **notes:** a magic string as a menu value — fine internally, but a port must not let it reach settings.

#### P-274 — Empty states exist for the per-model list and for auth
- **behaviour:** With no configured providers, step 1 shows a single entry `No models available` described `Log in to a provider or configure an API key first`.
- **area:** settings / empty states
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:634`, `:636`
- **how to observe it:** clear all provider config, then `/settings` → per-model thinking.
- **notes:** the description names the next action rather than merely reporting emptiness — Epsilon's §8 "every async view has states" is met here by being *actionable*.

#### P-275 — Warnings is a nested toggle list
- **behaviour:** A `Warnings` row opens a nested list toggling `Anthropic extra usage`.
- **area:** settings / diagnostics
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:594`, `:156`, `:593` (the row's own value is the literal string `configure`), `:597` (applied immediately, not on leaving)
- **how to observe it:** `/settings` → Warnings.
- **notes:** the row's displayed value is the word `configure` rather than the current state — a placeholder that reads like a value. A port should show the actual state.

#### P-276 — Enum settings are edited by label and mapped back
- **behaviour:** Quiet startup (three-valued), Default project trust (Ask/Always/Never), HTTP idle timeout, Transport, Cache warming, Mermaid, Double-escape action and Tree filter mode are all plain labelled lists whose labels are mapped back to values.
- **area:** settings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:558`, `:923-924`, `:49`, `:55`, `:929`, `:931` (an unmapped label is silently ignored), `:515`, `:508`, `:524`, `:537`, `:576`, `:586`
- **how to observe it:** `/settings` → Tree filter mode → `user-only`, then `/tree`.
- **notes:** the quiet-startup row stringifies its value for display but passes a real enum back (`:558` vs `:924`) — an asymmetry a port will get wrong if it stores the display string. The HTTP idle timeout description explicitly mentions local models that pause over five minutes (`:513`).

#### P-277 — Fullscreen-only settings are listed but marked as such
- **behaviour:** Exit output (transcript/resume-hint), scrollbar (auto/always/hidden), copy-on-select and wheel scroll lines are listed, with descriptions stating they have no effect in regular TUI mode.
- **area:** settings / fullscreen
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:712`, `:739`, `:722`, `:850` (Clear on shrink is described as possibly flickering)
- **how to observe it:** `/settings` → Fullscreen wheel scrolling.
- **notes:** the wheel-scroll list is `auto` plus a de-duplicated, sorted 1/2/3/5/10 merged with the current value (`settings-selector.ts:739-745`) — so an out-of-range current value stays selectable.

---

### Settings submenu primitives

#### P-278 — Submenus are titled, described and list-capped at ten rows
- **behaviour:** A submenu renders a title, an optional description and a list capped at `min(options.length, 10)` rows, with a footer reading `Enter to select · Esc to go back` (or `Type to filter · …` when searchable).
- **area:** settings / overlays
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-submenu.ts:88`, `:95`, `:120`, `:134`
- **how to observe it:** `/settings` → Theme.
- **notes:** the cap of 10 is a hard constant, not derived from terminal height — it will clip awkwardly on a short terminal and waste space on a tall one.

#### P-279 — Submenu filtering drops the cursor to row 0
- **behaviour:** With `searchable: true`, typing fuzzy-filters on `label + description`; navigation keys go to the list and everything else to the box, and the rebuilt list has an **empty preselect**.
- **area:** settings / overlays
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-submenu.ts:110`, `:115`, `:120`
- **how to observe it:** `/settings` → per-model thinking, type `gpt` — the cursor is on the first match.
- **notes:** contrast with the thinking selector, which tries to preserve the selection (P-263). Two submenu-like components, two policies.

#### P-280 — Multi-step submenus show `Step N/M` and Esc walks back one step
- **behaviour:** A multi-step submenu prefixes each description with `Step N/M · `, titles and descriptions may be functions of the accumulated selections, and Esc returns to the previous step, deleting that step's stored selection; Esc at step 0 cancels the submenu.
- **area:** settings / overlays
- **evidence:** `packages/coding-agent/src/modes/interactive/components/settings-submenu.ts:204`, `:149`, `:234`, `:236`
- **how to observe it:** `/settings` → per-model thinking → pick a model → Esc (back to step 1) → Esc (back to the settings list).
- **notes:** the `Step N/M ·` label is omitted entirely for a **single-step** stepped submenu (`:204`) — a small thing that keeps simple dialogs from reading as wizard steps.

---

### `pi config` — the resource manager

#### P-281 — `pi config` is a three-level resource hierarchy
- **behaviour:** `pi config` opens `Global Resources` over a search box with group headers, per-type subheaders (Extensions / Skills / Prompts / Themes) and resources.
- **area:** extensions / resources
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:209`, `:299`, `:166` (package groups sort before top-level, user before project, then by source), `:177`
- **how to observe it:** run `pi config`.
- **notes:** **this is a CLI command, not a slash command** — there is no `/config` in `BUILTIN_SLASH_COMMANDS` (`packages/coding-agent/src/core/slash-commands.ts:19-44`). A port that adds it to the composer changes the command surface.

#### P-282 — Space or Enter toggles a resource into settings
- **behaviour:** Toggling writes a `+pattern` or `-pattern` entry into the settings array for that resource type; existing `!`/`+`/`-` entries for the same pattern are removed first.
- **area:** extensions / resources
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:505`, `:557`, `:552`, `:574-582` (a per-type setter)
- **how to observe it:** `pi config`, Space on a skill row, then open the settings file.
- **notes:** the `+`/`-` prefix convention is what makes `defaultTools` modifiers and resource toggles read the same way (`settings-manager.ts:236-247`).

#### P-283 — Package resources are edited as filter objects
- **behaviour:** A resource belonging to a package toggles a pattern inside that package's entry, and the entry collapses back to a bare source string when it holds no filters.
- **area:** extensions / resources
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:586`, `:631`, `:869` (`getPackageResourcePattern`), `:732` (`autoload: false` entries are removed entirely)
- **how to observe it:** `pi config`, Space on a resource belonging to a package.
- **notes:** patterns are relative to the package base directory (`:869`), so the same absolute file has different patterns depending on who declares it.

#### P-284 — Tab switches the write scope
- **behaviour:** Tab flips Global ↔ Project; the header title, the settings-file hint and the toggle semantics all change, and the hint becomes `space cycle inherit/+/−`.
- **area:** extensions / resources
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:501`, `:942`, `:211`, `:929` (the hint is hidden entirely when project mode is unavailable)
- **how to observe it:** `pi config`, Tab.
- **notes:** the same Tab chord that switches session scope in `/resume` (P-230) — a consistent choice, not a coincidence.

#### P-285 — Project scope cycles three states, not two
- **behaviour:** In project scope Space cycles inherit → load → unload, choosing the next state from the inherited enabled value rather than a fixed order; checkboxes are `[+]` green, `[-]` warning, and inherited stays a plain `[x]`/`[ ]`.
- **area:** extensions / resources
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:739`, `:645`, `:648-650`, `:655` (suffix text `project load` / `project unload` / `inherited global`)
- **how to observe it:** `pi config`, Tab to Project, press Space repeatedly on a globally-enabled resource.
- **notes:** the cycle depends on the inherited value, so the *same* keypress produces different states for different rows. A port with a fixed three-state cycle will mislabel.

#### P-286 — Inherited globals are dimmed in project scope
- **behaviour:** Resources inherited from global settings render dim and their group header gains ` · inherited global`.
- **area:** extensions / resources
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:423`, `:663`, `:667` (dimming only while the override state is `inherit`), `:689`
- **how to observe it:** `pi config`, Tab to Project.
- **notes:** this is where "inherited" is communicated **without** relying on colour alone — the header suffix carries the same meaning.

#### P-287 — Ctrl+C in `pi config` exits the process
- **behaviour:** Esc closes the selector and returns to the shell; `Ctrl+C` calls `onExit`, which in the CLI host is `process.exit(0)`.
- **area:** extensions / lifecycle
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:493`, `:497`, `packages/coding-agent/src/cli/config-selector.ts:49`
- **how to observe it:** `pi config`, Ctrl+C.
- **notes:** **a key that kills the process from inside a UI overlay**, and `matchesKey(data, "ctrl+c")` is hard-coded, bypassing the keybinding registry. Epsilon's invariant 6 forbids reproducing that literally; the parity target is "Esc closes, Ctrl+C does not exit silently", not "Ctrl+C exits".

#### P-288 — Paging skips header rows, and the list height is chrome-derived
- **behaviour:** PageUp/PageDown jump by a page and then advance to the nearest **resource** row, skipping group and subgroup headers; the visible row count is `max(5, terminalHeight − 8)`.
- **area:** extensions / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:471`, `:315`, `:463`, `:467`, `:270`, `:272`
- **how to observe it:** `pi config`, PageDown repeatedly; then resize the terminal.
- **notes:** the constant 8 encodes a specific chrome layout (spacers, borders, two-line header). Changing the header without changing 8 silently clips rows — a magic number a port should derive instead.

#### P-289 — `pi config` search is substring, not fuzzy
- **behaviour:** Typing filters on display name, resource type or full path, keeps the matching group and subgroup headers visible, and the scroll indicator counts **items only** (`(3/17)`) rather than flat rows.
- **area:** extensions / overlays
- **evidence:** `packages/coding-agent/src/modes/interactive/components/config-selector.ts:341`, `:450`, `:342-345`, `:333`, `:329`
- **how to observe it:** `pi config`, type `skill`.
- **notes:** **substring, case-insensitive** — unlike every other selector in Pi, which fuzzy-matches. Matching once against a lowercased filter, not per keystroke against the raw query.

---

### `/tree` — the session tree selector

#### P-290 — The tree opens on the current leaf
- **behaviour:** `/tree` opens the `Session Tree` with the cursor on the current leaf, or on the nearest visible ancestor of it under the active filter.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:144`, `:153`, `:176`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5486`
- **how to observe it:** `/tree` mid-session.
- **notes:** the parent walk happens only within the **filtered** set, falling back to the last visible entry (`:176`). The initial filter comes from the `Tree filter mode` setting.

#### P-291 — Every entry kind has its own prefix
- **behaviour:** Rows are prefixed by entry kind: `user:`, `assistant:`, `[bash]: cmd`, `[read: path:1-20]`, `[write: p]`, `[edit: p]`, `[grep: /pat/ in dir]`, `[find: …]`, `[ls: …]`, `[compaction: Nk tokens]`, `[branch summary]:`, `[model: id]`, `[thinking: level]`, `[custom: type]`, `[context replace|omit: id]`, `[label: x]`, `[title: …]`.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:780`, `:946`, `:802` (`toolCallMap` resolves a tool result back to its call), `:980` (bash cut to 50 chars), `:998` (custom tools show truncated JSON), `:959` (`read` shows `:offset-limit` only when offset or limit was passed), `:948` (home abbreviated via `HOME`/`USERPROFILE`)
- **how to observe it:** `/tree` in a session that used read, bash and grep.
- **notes:** an orphaned tool result degrades to `[<toolName>]` — the resolver depends on finding the originating call, which is absent for a partially imported session.

#### P-292 — The active path is marked with `• `
- **behaviour:** Every entry on the path from root to the current leaf is prefixed with `• `.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:742`, `:180`, `:694`, `:751`
- **how to observe it:** `/tree` and follow the bullets.
- **notes:** three distinct markers coexist — `• ` for the active path, `›` for the selection cursor, and the dim `├─`/`└─` tree prefix. A port collapsing them will lose information.

#### P-293 — Five filter modes on direct keys
- **behaviour:** `Ctrl+D` sets the default view; `Ctrl+T` toggles hide-tool-results; `Ctrl+U` user messages only; `Ctrl+L` labelled entries only; `Ctrl+A` everything.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1047`, `:1052`, `:359` (default hides label, context edit, custom, model change, thinking change, session info), `:374` (`no-tools` is default minus `toolResult`), `:1049` (Ctrl+D is **absolute**) versus `:1054` (the others are toggles falling back to default)
- **how to observe it:** `/tree`, then Ctrl+U.
- **notes:** Ctrl+D sets; the rest toggle. Every filter change also clears folded branches (`:1050` etc.), so a filter change can visibly expand the tree.

#### P-294 — Ctrl+O cycles filters forward and back
- **behaviour:** `Ctrl+O` cycles forward and `Shift+Ctrl+O` backward through default → no-tools → user-only → labeled-only → all.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1079`, `:1072`, `:1074`, `:1081`, `packages/coding-agent/src/modes/interactive/components/settings-selector.ts:587`
- **how to observe it:** `/tree`, Ctrl+O five times.
- **notes:** the mode order is duplicated in **both** cycle branches and again in the settings selector — three copies to keep in sync. A port should declare it once (Epsilon invariant 4).

#### P-295 — The status line names the filter
- **behaviour:** The line under the tree always shows `(n/total)` plus a bracketed filter tag — `[no-tools]`, `[user]`, `[labeled]`, `[all]`, and nothing for default — and ` [+label time]` when timestamps are on.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:647`, `:765`, `:674` (the empty case prints `(0/0)` plus the same tags)
- **how to observe it:** `/tree` after Ctrl+U.
- **notes:** default has **no** tag, so a reader cannot tell "default" from "no filter applied" without knowing the convention.

#### P-296 — Labels are editable inline and removable by empty value
- **behaviour:** `Shift+L` opens an inline `Label (empty to remove)` input; submitting an empty value removes both the label and its timestamp; the label renders in warning colour before the entry text.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1092`, `:1322`, `:1412`, `:1424` (while the input is open the tree list is removed from the container), `:745`
- **how to observe it:** `/tree`, Shift+L, type a tag, Enter — the row shows `[tag]`.
- **notes:** "empty to remove" is the removal affordance; there is no separate delete key.

#### P-297 — Label timestamps have three date formats
- **behaviour:** `Shift+T` toggles showing when each label was set, rendered as `HH:MM` today, `M/D HH:MM` this year, `YY/M/D HH:MM` otherwise.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1097`, `:862`, `:641` (stamped on edit, cleared with the label)
- **how to observe it:** `/tree`, Shift+T.
- **notes:** three formats keyed on recency — a port that uses one format will look wrong for old sessions.

#### P-298 — Tree search drops control characters
- **behaviour:** Typing appends printable characters to the tree query; characters with codes `<32`, `0x7f` or `0x80-0x9f` are dropped, and backspace removes one character.
- **area:** sessions / tree / security
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1099`, `:1086`
- **how to observe it:** `/tree`, type `user`, Backspace.
- **notes:** **there is no `Input` widget** — the query is a raw string and the `Type to search:` line is display-only (`:1174`). Matching is substring, lowercased, whitespace-tokenised with AND semantics (`:337`, `:395`) and covers role, text, labels and per-type keywords (`:562`). The control-character filter is narrower than Epsilon's §6 bidi requirement.

#### P-299 — Esc is two-stage in the tree
- **behaviour:** Esc clears the search query **and** all folded branches first; a second Esc exits the overlay.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1039-1043`
- **how to observe it:** `/tree`, type a query, Esc (list unfiltered, overlay open), Esc (closed).
- **notes:** unfolded state is lost along with the query — deliberate, and worth restating because it looks like a bug.

#### P-300 — Fold, unfold and branch-jump share two keys each
- **behaviour:** `Ctrl+Left`/`Alt+Left` folds the selected branch if it is foldable and not already folded, otherwise jumps to the previous branch-segment start; `Ctrl+Right`/`Alt+Right` unfolds or jumps to the next segment start.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1010`, `:1018`, `:1117` (foldability needs visible children **and** root-or-segment-start), `:1139` (the down-jump always follows the first child), `packages/coding-agent/src/core/keybindings.ts:150-157` (macOS puts Alt first)
- **how to observe it:** `/tree`, Ctrl+Left on a branch.
- **notes:** one key, two meanings decided by state. The chord order differs on macOS, which changes which one a user presses first.

#### P-301 — Folded branches use `⊞`, expanded use `⊟`
- **behaviour:** A folded branch renders `⊞` where an expanded one renders `⊟`; folded roots without a connector get a leading `⊞ ` marker, and a non-foldable node shows `─`.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:726`, `:738`, `:727`, `:401-411` (descendants are removed before the visual structure is recomputed)
- **how to observe it:** `/tree`, fold, look at `└⊞` versus `└⊟`.
- **notes:** the glyph, not the branch count, is the only collapsed indicator.

#### P-302 — The tree has a horizontal viewport that protects the gutter
- **behaviour:** Row bodies shift left only when the selected row's anchor would sit too close to the right edge, and the 2-column tree gutter is never clipped.
- **area:** sessions / tree / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:62`, `:75`, `:85`, `:71-79` (threshold `min(20, max(4, viewport/3))`, retained context `min(12, max(2, viewport/4))`), `:88` (a reset sequence is appended after clipping)
- **how to observe it:** `/tree`, select a deeply indented node in a narrow terminal.
- **notes:** every row is re-clipped with the same offset so alignment survives; the thresholds are computed from the viewport rather than fixed.

#### P-303 — The active branch sorts first at every level
- **behaviour:** The subtree containing the current leaf is ordered before its siblings at every level.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:245`, `:274`, `:217-240` (an explicit iterative pre-order plus reverse pass, to avoid stack overflow on deep sessions)
- **how to observe it:** `/tree` after branching — the live branch is adjacent to its parent.
- **notes:** the iterative walk is a deliberate stack-depth decision; a recursive implementation in Rust would be fine, but the *ordering guarantee* is what matters.

#### P-304 — Indentation grows only at branch points
- **behaviour:** Indent grows by one level only at branch points and for the first generation after a branch, so a single-child chain stays flat; multiple roots are treated as children of a virtual branching root.
- **area:** sessions / tree / layout
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:288`, `:436` (indentation is **recomputed** against the visible tree), `:448` (descendants reattach to the nearest visible ancestor), `:479` (`multipleRoots` is re-evaluated)
- **how to observe it:** `/tree` in a linear session — no cumulative right drift.
- **notes:** because filtering can hide intermediate entries, indentation must be recomputed rather than inherited. A port that caches indentation per entry will render a collapsed filter wrong.

#### P-305 — Tool-only assistant messages are hidden from the tree
- **behaviour:** An assistant message with no text and a normal stop reason is dropped from the tree, unless it is the current leaf or carries an error/aborted stop reason.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:344`, `:351`, `:341` (`usage` entries are always dropped), `:792`, `:795` (aborted shows `(aborted)`, errored shows the message truncated to 80 chars), `:798` (`(no content)`), `:888` (message text cut to 200 chars)
- **how to observe it:** `/tree` in a session that was mostly tool calls — the list is much shorter than the transcript.
- **notes:** the tree is a **navigation** view, not a transcript; dropping bookkeeping entries is what makes it navigable.

#### P-306 — Left and Right also page in the tree
- **behaviour:** `tui.select.pageUp`/`pageDown` **and** `tui.editor.cursorLeft`/`cursorRight` both page by `maxVisibleLines`.
- **area:** sessions / tree / keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1026`, `:1029`, `:1007`, `:1009` (arrows **wrap** here), `:1370` (viewport height `max(5, terminalHeight / 2)`)
- **how to observe it:** `/tree`, press Left/Right or PageDown.
- **notes:** cursor-left/right paging is surprising and undocumented; it exists so the left-hand fold keys have a paging counterpart. And this selector **wraps** at the ends, unlike the session selector which clamps — another per-selector divergence.

#### P-307 — Ctrl+X copies the full entry text, not the preview
- **behaviour:** `Ctrl+X` copies the selected entry's **full** text rather than the 200-character preview shown on screen.
- **area:** sessions / tree / clipboard
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1037`, `:904`, `:930` (an entry with no copyable text sends `undefined`), `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5616-5627` (`onCopy`, `Selected entry has no text to copy`)
- **how to observe it:** `/tree`, Ctrl+X on a long message.
- **notes:** the discrepancy between preview and copy is deliberate and worth stating in a port's own help, or users will think the copy failed.

#### P-308 — An empty tree closes itself after 100 ms
- **behaviour:** A tree with no entries cancels the overlay automatically after 100 ms.
- **area:** sessions / tree
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1395`, and the host guard at `packages/coding-agent/src/modes/interactive/interactive-mode.ts:5488-5491` (`No entries in session`)
- **how to observe it:** `/tree` on a fresh session — the overlay flashes and closes.
- **notes:** the same 100 ms auto-cancel is used by the fork picker (`user-message-selector.ts:147`). Showing an empty tree for a beat is less jarring than an instant flash.

#### P-309 — The tree footer lists every binding with arrow glyphs
- **behaviour:** The tree's help footer lists each binding, rewriting `pageUp`→`pgup` and `up`→`↑`, and merges chords sharing a prefix into one display (`ctrl+left/right`).
- **area:** sessions / tree / keybindings
- **evidence:** `packages/coding-agent/src/modes/interactive/components/tree-selector.ts:1254`, `:1263` (`compactRawKeys`), `:1395`
- **how to observe it:** `/tree`, read the bottom line.
- **notes:** keys are read from the live table (`:1254`), so the footer follows a rebind.

---

### `/fork` — the user-message selector

#### P-310 — The fork picker explains what it will do
- **behaviour:** `/fork` opens `Fork from Message` with the explanation `Select a user message to copy the active path up to that point into a new session`, cursor on the most recent message.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/user-message-selector.ts:123`, `packages/coding-agent/src/modes/interactive/interactive-mode.ts:3214`, `:5432`, `:5427` (the host short-circuits an empty list)
- **how to observe it:** `/fork` in a session with several turns.
- **notes:** the in-component auto-cancel is a fallback; the host normally refuses to open it at all.

#### P-311 — Fork rows are three lines each, ten visible
- **behaviour:** Each message occupies three lines — the text (newlines flattened, truncated to width), a `Message N of M` caption, and a blank; ten messages are visible with a `(n/total)` indicator.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/user-message-selector.ts:54`, `:42`, `:73`, `:19` (`maxVisible` is a hard-coded 10)
- **how to observe it:** `/fork` in a long session.
- **notes:** control characters are **not** filtered here, unlike the session selector (`session-selector.ts:470`) — an inconsistency in untrusted-text handling between two components in the same directory.

#### P-312 — Fork navigation wraps
- **behaviour:** Up from the top wraps to the newest message and Down from the bottom wraps to the oldest.
- **area:** sessions
- **evidence:** `packages/coding-agent/src/modes/interactive/components/user-message-selector.ts:84`, `:88`, contrasted with `packages/coding-agent/src/modes/interactive/components/oauth-selector.ts:198` (which clamps)
- **how to observe it:** `/fork`, press Up from the default position.
- **notes:** **wrapping and clamping both exist in Pi**, in sibling components. A port must choose per selector or it will differ from Pi in half of them.

---

### Auth and login overlays

#### P-313 — Provider rows carry configured/not-configured badges
- **behaviour:** Each provider row carries a suffix: `• not configured`, `✓ configured`, `✓ env: NAME`, or a warning-coloured `<type> configured` when the stored credential's type differs from the row's.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/oauth-selector.ts:36`, `:47`, `:49-52` (an env-style source renders as `env: NAME` only when it matches `^[A-Z][A-Z0-9_]*$`), `:42-46` (`OAuth` and `stored credential` render as plain `✓ configured`)
- **how to observe it:** `/logout` with one provider set by an env var and one unconfigured.
- **notes:** the type-mismatch case is a genuine diagnostic — it means the row says "login with an API key" while an OAuth token is stored.

#### P-314 — Auth-type tags appear only when the list is mixed
- **behaviour:** The `[api key]` / `[subscription]` / `[account]` tag is appended only when the list contains more than one auth type.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/oauth-selector.ts:92`, `:160`, `:32` (`subscription: false` downgrades "subscription" to "account")
- **how to observe it:** `/login` after filtering to one provider type — the tag disappears.
- **notes:** conditional tags keep a single-type list uncluttered; a port that always shows them will be noisier than Pi.

#### P-315 — Three distinct auth empty states
- **behaviour:** The messages are `No providers available` (login), `No providers logged in. Use /login first.` (logout) and `No matching providers` (filter matched nothing).
- **area:** model / auth / empty states
- **evidence:** `packages/coding-agent/src/modes/interactive/components/oauth-selector.ts:182`, `:186`
- **how to observe it:** type a nonsense query in `/logout`.
- **notes:** three different conditions get three different words — "nothing configured", "nothing to remove" and "your filter matched nothing" are genuinely different states.

#### P-316 — The Radius login option shimmers
- **behaviour:** The `Sign in with Radius` option animates through four brand colours in 4-character bands at 10 chars/second, redrawing every 50 ms — but only while that row is selected.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/radius-login-selector.ts:11`, `:18`, `:72`, `:29`, `:32`
- **how to observe it:** `/login` with Radius available; move the cursor on and off the option.
- **notes:** the animation is driven by an `unref()`'d interval that only calls `requestRender` while animating (`:72-75`), and is cleared on select, cancel and dispose (`:63`, `:92`, `:98`). Colour mixing is smoothstep in sRGB — *not* perceptually uniform, and not WCAG-checked.

#### P-317 — The shimmer is a rendered-line patch, not a widget
- **behaviour:** The component re-renders the parent list, finds the exact plain `→<label>` line in the output and replaces it with an animated version; if the line is not found it renders normally.
- **area:** model / auth / rendering
- **evidence:** `packages/coding-agent/src/modes/interactive/components/radius-login-selector.ts:78`, `:81`, `:41-44` (the file documents the brittleness itself)
- **how to observe it:** `/login` in a very narrow terminal so the Radius label wraps — the animation silently disappears.
- **notes:** string-matching the rendered output is brittle **by construction**. Epsilon's §8 reduced-motion requirement means a port should render this as a widget that can be switched off, not as a post-render string substitution.

#### P-318 — The OAuth login dialog opens the browser automatically
- **behaviour:** During OAuth the composer is replaced by a `Login to <provider>` panel that renders the URL as an OSC-8 hyperlink, adds a platform-dependent click hint, and **opens the browser without a keypress**.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/login-dialog.ts:96`, `:111`, `:102`, `:124` (hint is `Cmd+click to open` on darwin, `Ctrl+click to open` elsewhere), `:106` (optional instructions in warning colour), `:73` (exposes an `AbortSignal`)
- **how to observe it:** `/login <provider>` that needs a browser callback.
- **notes:** **auto-opening a browser with no prompt** is a behaviour a port should reconsider — Epsilon's §6 treats "no silent side effects" as a rule. Hyperlinks must also degrade gracefully where OSC 8 is unsupported.

#### P-319 — The device-code flow does not open a browser
- **behaviour:** A device-code login shows the verification URI as a hyperlink plus `Enter code: XXXX-XXXX` and does not open a browser.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/login-dialog.ts:118`, `:128`, `:152-153` (`showAuth`/`showDeviceCode` **clear** the container) versus `:136-140` (`showPrompt`/`showInfo`/`showWaiting`/`showProgress` **append**)
- **how to observe it:** `/login <provider>` that uses device code.
- **notes:** the clear-vs-append split is deliberate so the URL stays on screen while later steps append below it.

#### P-320 — A submitted login input is echoed in place
- **behaviour:** A submitted input is replaced in place with a `> value` line so what the user typed stays visible; Esc aborts the signal, rejects with `Login cancelled` and reports `onComplete(false, "Login cancelled")`.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/login-dialog.ts:77`, `:83`, `:225`, `:65`
- **how to observe it:** any OAuth step, press Esc.
- **notes:** the cancel path is reachable from **two** entry points for the same chord (`Input.onEscape` at `:65` and the component's own match at `:225`). Waiting/info panels show `(esc to cancel)` / `(esc to close)` (`:210`, `:141`, `:163`, `:199`).

#### P-321 — The auth selector clamps, the model selector does not
- **behaviour:** The auth provider list's Up/Down stop at the ends, and the selected index is clamped after every filter change so it does not reset to the top.
- **area:** model / auth
- **evidence:** `packages/coding-agent/src/modes/interactive/components/oauth-selector.ts:198`, `:204`, `:139`, `:146` (8 rows visible)
- **how to observe it:** `/logout`, press Up at the top; then filter and check the cursor stays put.
- **notes:** **three different navigation policies** across three pickers: clamps (auth), clamps but keeps position (model), wraps (fork/tree). See P-252 and P-312.

---

### Two components that exist but are never opened

#### P-322 — `ThemeSelectorComponent` has no production caller
- **behaviour:** A bordered theme list capped at 10 rows, marking the current theme `(current)`, pre-selecting it and live-previewing on cursor move — reachable only if an extension instantiates it.
- **area:** themes
- **evidence:** `packages/coding-agent/src/modes/interactive/components/theme-selector.ts:31`, `:54`, `packages/coding-agent/src/modes/interactive/components/index.ts:31`, `packages/coding-agent/src/index.ts:460`
- **how to observe it:** not reachable from any Pi command; instantiate it from an extension.
- **notes:** **grep finds only the definition and the two re-exports.** The theme Pi actually shows the user is the theme submenu inside `settings-selector.ts` (P-269). A porter building a `/theme` command from this file would be implementing dead code — Epsilon's invariant 7 says a control that nothing opens must not be advertised.

#### P-323 — `ShowImagesSelectorComponent` has no production caller either
- **behaviour:** A two-row Yes/No list described `Show images inline in terminal` / `Show text placeholder instead`, pre-selecting the current boolean.
- **area:** files / terminal capability
- **evidence:** `packages/coding-agent/src/modes/interactive/components/show-images-selector.ts:19`, `:31`, `packages/coding-agent/src/modes/interactive/components/index.ts:29`, `packages/coding-agent/src/index.ts:458`
- **how to observe it:** not reachable from any Pi command.
- **notes:** the user-facing equivalent is the `Show images` row in `/settings` (P-267), which appears only when the terminal supports images. Same dead-code finding as P-322.

---

## AA. Not yet inventoried

Stated plainly, with what I saw and why. Nothing here is a silent gap.

### AA-001 — `packages/tui/src/` in depth — **PARTIALLY DONE, delegation FAILED**

A delegated pass over `packages/tui` **failed before delivering its behaviour rows.** What survived and is in this file:

- **The complete `TUI_KEYBINDINGS` table**, delivered as an interim before the failure. I re-read `packages/tui/src/keybindings.ts:71-210` myself and verified all 47 entries character-for-character. Integrated as **L-000 to L-003**.
- **The text-measurement core**, which I read and wrote up myself afterwards: **T-001 to T-012** cover `visibleWidth`, `graphemeWidth`, tab-as-3-columns, ANSI/OSC/APC stripping, `truncateToWidth`, `sliceByColumn`/`sliceWithWidth`, `wrapTextWithAnsi`, `breakLongWord`, the ASCII fast path and its 512-entry cache, `applyBackgroundToLine`, Thai/Lao decomposition and the CJK autocomplete boundary. All from `packages/tui/src/utils.ts`.
- The viewport and renderer composition, covered earlier as P-016 and P-217 to P-220.

**Still not read at all**, with the files that hold them:
- `packages/tui/src/editor.ts` (2112 lines) — the editor: cursor, prompt history, undo, the autocomplete popup, bracketed paste, `CURSOR_MARKER` and IME. **The largest single gap.**
- `packages/tui/src/keys.ts` (1286 lines) — key parsing and terminal protocol negotiation.
- `packages/tui/src/tui-alt-screen.ts` (1634 lines) — search, text selection, overlays, copy-on-select, wheel handling.
- `packages/tui/src/tui-main-screen.ts` (589) and `tui.ts` (1337) — differential rendering, focus, resize, terminal title, progress.
- `packages/tui/src/terminal.ts` (479) and `terminal-colors.ts` (81) — capability detection, raw mode, the DA1 colour query behind P-011.
- `packages/tui/src/terminal-image.ts` (628) — kitty/iterm2 image protocols, behind P-149 and P-150.
- `packages/tui/src/markdown.ts` (897) and `latex.ts` (1428) — markdown and LaTeX rendering.
- `packages/tui/src/autocomplete.ts` (740) and `fuzzy.ts` (111) — how the `/` menu ranks, behind P-043 to P-045.
- `packages/tui/src/scroll-view.ts` (198), `select-list.ts` (234), `settings-list.ts` (288), `image.ts` (111), `loader.ts` (88), `box.ts` (141).

**How to close it:** re-run the delegation with `utils.ts` and `keybindings.ts` removed from scope (both are now done), and ask for `editor.ts`, `keys.ts`, `terminal.ts` and `tui-alt-screen.ts` specifically. Those four cover most of what is left.

### AA-002 — `components/armin.ts`, `components/daxnuts.ts`, `components/pi-logo-animation.ts`
Covered by one row each (P-221, P-222, P-223) plus P-224. Only the entry and exit paths of the 1209-line animation were read; the ray-caster/shader middle is animation detail behind P-223. **Deliberately not a parity target.**

### AA-003 — `src/experimental/`
Not inventoried. `src/experimental/client-tui.ts` is a **second, separate TUI** (27 kB) belonging to the experimental remote-client architecture, not to the interactive mode this matrix covers. See Y-006. If Epsilon adopts a remote-execution model, that tree needs its own inventory.

### AA-004 — `src/core/agent-session.ts` (158 kB)
Read only where the interactive mode calls into it. Its own event catalogue, queue semantics and compaction behaviour are inventoried **as the TUI observes them** (P-030 to P-033, P-049 to P-055, P-186 to P-192), not as core behaviour.

### AA-005 — Untrusted-text sanitisation
Pi strips control characters in **two** places I found (`session-selector.ts:470`, `tree-selector.ts:1086`) and not in at least one sibling (`user-message-selector.ts:54`, which flattens newlines but filters no control characters). I did **not** find a single central sanitiser for display text. Epsilon's §6 requires stripping control **and bidi** characters at every screen boundary. **I did not exhaustively grep for a central sanitiser**, so I am not claiming Pi lacks one — only that I did not find it, and that the component-level behaviour I did read is inconsistent. This is a finding to verify before relying on it.

### AA-006 — Unreachable code inside inventoried files
Reported so a porter does not port them as features:
- `SessionSelectorComponent.canRename` is written (`session-selector.ts:709`, `:779`) and never read.
- `SessionList.onExit` is assigned by the host (`session-selector.ts:810`) but no key path calls it — unlike `config-selector.ts:497`, which does call its own `onExit`.
- `ThemeSelectorComponent` and `ShowImagesSelectorComponent` have no production caller (P-322, P-323).
- `ExtensionInputComponent`'s `placeholder` argument is accepted and discarded (P-214).
- `bash-execution.ts` honours `excludeFromContext` in its constructor but not on rebuild (P-153) — a live defect, not dead code.

### AA-007 — Reference defects found and recorded, not silently copied
Each is cited in its row. Consolidated here because a porter should decide rather than inherit:
1. **P-153** — `!!` bash cards lose their dimmed border on the first rebuild.
2. **P-214** — the extension input ignores its `placeholder`.
3. **P-232** — the session selector's Ctrl+N branch reads a different keybinding source than every other branch in the same file.
4. **P-287** — `Ctrl+C` inside `pi config` calls `process.exit(0)`, hard-coded past the keybinding registry. Epsilon invariant 6 forbids reproducing this.
5. **P-251** — typing a single `d` in the model selector reshuffles the whole list.
6. **P-306** — Left/Right page the tree, and are documented nowhere.
7. **P-245** — load-failure timeout is 4 s while delete-failure timeout is 3 s in the same component.
8. **P-137, P-202** — tool-card state and footer context pressure are carried by **colour alone**, which is below Epsilon's own §8 floor.

---

## AB. Ten behaviours a porter would otherwise miss

Shortest possible list of the things most likely to be got wrong from memory.

1. **The default TUI mode is `fullscreen`, not `regular`** — anything that is not the literal string `"regular"` means fullscreen (`core/settings-manager.ts:1348-1350`). Two entirely different renderers with different capabilities.
2. **The default theme is `system`, not `dark`** — generated from the terminal's reported colours, and grey for up to 100 ms at startup (`theme/theme.ts:757`, `theme-controller.ts:25`).
3. **Pi has no permission system at all** — by design, stated in its own README (`README.md:42`). The only gate is *project* trust, which decides whether `.pi` resources load, not whether tools may run.
4. **Enter steers, Alt+Enter follows up** — two genuinely different semantics on two keys (`interactive-mode.ts:3322` versus `:4409`).
5. **Escape is a four-way priority switch that gets *replaced*, not chained**, during compaction and retry (`interactive-mode.ts:3011-3037`, `:3614-3617`, `:3677-3680`).
6. **Truncation policy differs per surface**: generic tool keeps the **first** 10 lines, `!` bash keeps the **last** 20 *visual* lines, the bash tool keeps the **last** 5 (`tool-execution.ts:40`, `bash-execution.ts:131`, `core/tools/renderers/bash.ts:18`).
7. **`quietStartup` is three-valued** (`true` / `"header"` / `false`), and `outputPad` is 0-or-1 with any other value coerced to 1 (`settings-manager.ts:1089`, `:1488`).
8. **In fullscreen, `PageUp`/`Home`/`End` scroll the transcript, not the composer** — the alt-screen table deliberately shadows the editor table (`packages/tui/src/keybindings.ts:159`).
9. **Six shipped actions have no default key at all** and are therefore invisible in `/hotkeys` (`packages/tui/src/keybindings.ts:74-81`, `:168-183`).
10. **`pi config` is a CLI command, not a slash command** — there is no `/config` in the built-in table (`core/slash-commands.ts:19-44`), and `Ctrl+C` inside it exits the process (`config-selector.ts:497`).

---

## AC. TUI library: text measurement and slicing

*(Read directly by me after the delegated `packages/tui` pass failed. Paths relative to `references/pi-1.0.0`. This is the highest-value section for a Rust port: every width bug in a terminal UI lives here.)*

### T-001 — Width is measured in grapheme clusters, not code units
- **behaviour:** Every width computation segments text into grapheme clusters first, then sums a per-cluster cell count.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:250-310` (`visibleWidth`, segmenting at `:296`), `packages/tui/src/utils.ts:182-245` (`graphemeWidth`, the per-cluster rules)
- **how to observe it:** a footer row containing a flag emoji or a combining accent; the layout does not drift.
- **notes:** a Rust port must use a grapheme segmenter (`unicode-segmentation`), **not** `chars().count()`. `graphemeWidth`'s order of rules matters: printable ASCII → 1 (`:184-186`); tab → 3 (`:187`); terminal spacing marks → count of code points (`:190-192`); zero-width clusters → 0 (`:195-197`); RGI emoji → 2 (`:200-202`); regional indicator symbols → **2 even when isolated** (`:214-216`); otherwise East Asian Width plus corrections for marks, halfwidth/fullwidth forms and Thai/Lao AM vowels (`:218-243`).

### T-002 — A tab counts as three columns everywhere
- **behaviour:** A literal tab is measured as **3** columns, not as "advance to the next tab stop".
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:187` (`if (code === 0x09) return 3`), `packages/tui/src/utils.ts:271` (`visibleWidth` expands tabs to three spaces), `packages/tui/src/utils.ts:441` (the ASCII fast path adds 3)
- **how to observe it:** output containing tabs; the layout never wraps because of a tab stop.
- **notes:** this is a **deliberate simplification** — `normalizeTerminalOutput` expands visible tabs to three spaces precisely so "terminal tab stops cannot wrap a logical line" (`utils.ts:386-390`, `:412`). The same value (three spaces) is used for diff expansion (`diff.ts:17`, P-164). A port using 8, or using real tab stops, will not match Pi's layout.

### T-003 — ANSI, OSC and APC sequences are stripped before measuring
- **behaviour:** CSI styling/cursor codes, OSC hyperlinks and OSC 133 prompt markers, and APC sequences such as `CURSOR_MARKER` are all removed before width is computed.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:273-292` (the single-pass strip in `visibleWidth`), `packages/tui/src/utils.ts:452-462` (`ansiCodeLength`, CSI terminated by `m`, `G`, `K`, `H`, `J`), `packages/tui/src/utils.ts:313-327` (`stripTerminalSequences`)
- **how to observe it:** styled output measures the same as unstyled.
- **notes:** the comment is explicit that this covers "CSI styling/cursor codes, OSC hyperlinks and prompt markers, and APC sequences like CURSOR_MARKER" (`utils.ts:275-277`). A port that only strips CSI will mis-measure every hyperlink.

### T-004 — `truncateToWidth` measures, clips by grapheme, then closes styling
- **behaviour:** Truncation clips by visible width, appends an ellipsis, **closes any open OSC 8 hyperlink**, and appends a reset — so a truncated styled string never bleeds its style or link into the next line.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:1121-1257` (`truncateToWidth`), `packages/tui/src/utils.ts:155-175` (`finalizeTruncatedResult`; `hyperlinkClose` at `:164`, reset at `:169`), `packages/coding-agent/docs/tui.md:26` (use this instead of hand-rolling)
- **how to observe it:** truncate a line containing a hyperlink; the link stops at the cut.
- **notes:** five behaviours a porter will miss. (a) If the text **fits**, it is returned unpadded-with-ellipsis — truncation is detected by scanning past the target (`utils.ts:1178-1183`, `:1252-1254`), not by comparing lengths up front. (b) The ellipsis is itself clipped if it is wider than `maxWidth` (`:1136-1147`). (c) ANSI codes **pending** when the cut lands are dropped, so the trailing escape run is not left dangling (`:1234-1235`). (d) `pad: true` pads to exactly `maxWidth` **after** the reset (`:174`). (e) The whole-string fast path only applies to printable ASCII (`:1149-1155`); anything else takes the grapheme walk.

### T-005 — `sliceByColumn` slices by visible column, not by byte or char
- **behaviour:** A range of visible columns can be extracted from a styled line, keeping the ANSI codes that were active at the boundary; a `strict` flag excludes wide characters that would extend past the range.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:1263-1265` (`sliceByColumn`), `packages/tui/src/utils.ts:1268-1316` (`sliceWithWidth`; the `fits` test at `:1300`, the ordering comment at `:1286`)
- **how to observe it:** the tree selector's horizontal viewport clipping (P-302).
- **notes:** the ordering comment is a real constraint: "codes from before the range must precede codes at the boundary" (`:1286`). Getting that wrong produces text in the wrong colour. `sliceByColumn` returns only the text; `sliceWithWidth` also returns the achieved width, which matters because a non-strict slice can be **wider** than requested.

### T-006 — Wrapping carries ANSI state across the break and across newlines
- **behaviour:** `wrapTextWithAnsi` tracks active styling across wrapped lines and across literal newlines, re-emitting the active codes at the start of each continuation line.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:900-923` (`wrapTextWithAnsi`; the prefix at `:913`), `packages/tui/src/utils.ts:925-1004` (`wrapSingleLine`, which trims and re-applies the tracker at `:982-986`)
- **how to observe it:** a bold markdown paragraph wrapping at 80 columns; every line is bold, none bleeds.
- **notes:** three subtleties. (a) At a wrap point Pi adds only an **underline** reset, deliberately "not full reset, to preserve background" (`:973-975`) — a full reset here would drop a background colour mid-paragraph. (b) A new line never **starts** with whitespace (`:980-983`). (c) The final line gets **no** reset — the caller owns it (`:997-1000`).

### T-007 — A token longer than the width is broken by grapheme, not by character
- **behaviour:** A single token wider than the line — a URL, a long path — is broken across lines one grapheme cluster at a time rather than overflowing or being dropped.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:947-967` (the long-token branch), `packages/tui/src/utils.ts:1022-1045+` (`breakLongWord`, which separates ANSI from visible content first)
- **how to observe it:** a very long URL in a tool result at 80 columns.
- **notes:** `breakLongWord` splits the token into `{type:"ansi"}` and `{type:"grapheme"}` segments first (`utils.ts:1030-1045`), because ANSI codes are not graphemes. A port that splits on `char_indices` will cut escape sequences in half — this is the single most common terminal-rendering bug.

### T-008 — There is an ASCII fast path, and it exists for a stated reason
- **behaviour:** `visibleWidth` short-circuits printable ASCII, tabs and ANSI sequences without allocating or segmenting; everything else goes through grapheme segmentation and a 512-entry cache.
- **area:** library / performance
- **evidence:** `packages/tui/src/utils.ts:255-261` (the fast path and its comment), `packages/tui/src/utils.ts:430-450` (`asciiVisibleWidth`, returning `-1` for anything else), `packages/tui/src/utils.ts:50-52` and `:301-307` (the bounded cache with FIFO eviction)
- **how to observe it:** scrolling a transcript; frame cost stays flat.
- **notes:** the comment states the reason: "Styled lines take this path, so re-rendering after a theme change does not run grapheme segmentation on every line" (`utils.ts:255-256`). Epsilon's §8 performance rule points at exactly this. The cache is **bounded at 512 with first-in eviction** (`:301-306`) — unbounded caching of every rendered line would be a memory leak.

### T-009 — Background fill is applied after padding, to the padded string
- **behaviour:** `applyBackgroundToLine` measures the line, appends spaces to reach the requested width, and only then applies the background — so the fill covers the full width rather than just the text.
- **area:** library / text
- **evidence:** `packages/tui/src/utils.ts:1099-1108`
- **how to observe it:** the user-message block's background reaching the right edge (P-133).
- **notes:** the order is load-bearing. Applying the background first and padding after leaves unpainted trailing cells — which is exactly the bug P-133 warns about when it explains why the user block must not also be wrapped in a `Box`.

### T-010 — Thai and Lao precomposed vowels are decomposed before output
- **behaviour:** Output is normalised so precomposed Thai/Lao `AM` vowels are replaced by their compatibility decompositions, which have the same cell width but avoid stale-cell artefacts during differential repaint.
- **area:** library / text / platform
- **evidence:** `packages/tui/src/utils.ts:383-393` (the rationale in the doc comment), `packages/tui/src/utils.ts:394-416` (`normalizeTerminalOutput`), `packages/tui/src/utils.ts:391-392` (the regexes)
- **how to observe it:** Thai text in a streaming message; no ghost characters on redraw.
- **notes:** a real rendering fix, not a normalisation nicety — the comment says those terminals "render precomposed Thai/Lao AM vowels inconsistently during differential repaint" (`utils.ts:385-387`). A port using ratatui's own normalisation may or may not do this; worth checking explicitly.

### T-011 — CJK is a word character, so autocomplete does not break inside it
- **behaviour:** The autocomplete boundary treats Han, Hiragana, Katakana, Hangul and Bopomofo characters as **word** characters — only CJK punctuation separates prose from a completion.
- **area:** library / autocomplete
- **evidence:** `packages/tui/src/utils.ts:54-55` (`cjkBreakRegex`), `packages/tui/src/utils.ts:57-63` (the comment "CJK letters remain part of words and paths; only punctuation can separate prose from completions" and the derived separator/boundary regexes)
- **how to observe it:** typing a `@` path completion containing Chinese characters; the whole segment completes.
- **notes:** without this, autocomplete splits every CJK character. A Rust port that uses only `char::is_alphanumeric` will get this wrong, and `is_alphanumeric` is true for CJK — the missing half is the punctuation set (`，．：；！？（）［］｛｝""''…—`, `utils.ts:59`).

### T-012 — Hyperlinks can be located by visible column
- **behaviour:** Given a rendered line and a visible column, the OSC 8 hyperlink covering that column can be recovered — which is how clicks are mapped to URLs.
- **area:** library / terminal
- **evidence:** `packages/tui/src/utils.ts:359-381` (`getOsc8LinkAtColumn`), `packages/tui/src/utils.ts:335-356` (`getGraphemeCellRange`, the inverse lookup)
- **how to observe it:** fullscreen mode, click a link in the transcript.
- **notes:** the tracker must reset `activeUrl` to `undefined` on a hyperlink-close with an empty target (`utils.ts:367`) — otherwise a closed link keeps firing. The docs note that **OSC 8 links take precedence over enclosing click regions** (`packages/coding-agent/docs/tui.md:60`), so this lookup runs before mouse-region hit testing.

<!-- APPEND-MARKER -->

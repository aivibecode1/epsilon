# Pinning-quality sample — 2026-10-09

**The stream this serves:** goal success criterion 4's closing duty and plan 026 R8. The 2026-10-09
sweep measured that every `done` row's *named test exists* (119/119). This is the stronger
question — the F5 class the 2026-10-06 objective review named: **does each named test actually
move the behaviour its row claims?** A row whose test exists but asserts something else is the
failure this repository has been bitten by before (`P-252` claimed a test that never typed a
query; `P-313` claimed tests that never look at a provider row), and no check can see it: the
citation checker resolves `file:line`, the count checker resolves arithmetic, and neither reads
the assertion.

## Method

Every fifth `done` row in both matrices, sampled from the matrices' own tables (the same
extraction the count checker uses — escaped pipes handled, id-anchored lines only). For each:
the `verified` cell's test names were located in the tree, then the test bodies were read and
judged against the row's `behaviour` cell. Existence was 27/27; judgement is below.

## The sample

### Pi (`parity/pi-tui.md`, 61 done, 12 sampled)

| Row | Behaviour the row claims | Test named | Verdict |
|---|---|---|---|
| P-222 | `bang_prefix_switches_the_footer_to_bash_mode` — types `!ls` through the real key path, asserts `bash` on the frame, backspaces, asserts it is gone | pins |
| P-211 | `up_and_down_walk_prompt_history` (two prompts typed and submitted, walk and empty-restore) + `ctrl_up_jumps_to_prompts_instead_of_recalling` (viewport moves, draft untouched) | pins |
| P-050 | `a_tool_result_is_a_card_whose_title_carries_name_status_and_duration`, `a_completed_tool_call_is_drawn_once_on_a_real_frame`, `a_todo_result_settles_its_call_and_nests_its_checklist` — the todo one drives the real event stream and asserts the settled `✓ todo (1/3 done) · ok` title and the nested checklist body | pins |
| P-133 | `a_user_turn_carries_its_gutter_and_band_on_a_real_frame` — asserts the `▎` gutter marker on the user row, and one marker not two | pins |
| P-078 | `export_command_writes_html_and_json` (writes the files, asserts `<html` and the message text, the run report) + `export_bare_defaults_to_html_in_exports`, `export_json_fmt_and_jsonl_suffix` (destinations and inference, dir-mutation control) | pins |
| P-136 | `tool_cards_are_separated_by_blank_rows` — blank-before-card on the frame, never two blanks | pins |
| P-159 | `truncated_shell_result_names_its_spill_file` — names the spill file, and the control that a small output grows none | pins |
| P-174 | `a_long_diff_is_capped_and_says_which_key_opens_it` — the affordance names the key and the true hidden count | pins |
| T-009 | same gutter/band frame test plus `a_wrapped_banded_message_keeps_every_row_inside_the_frame` (every wrapped row banded, continuation rows do not repeat the marker) | pins |
| P-204 | `nothing_is_drawn_past_the_row_width` (measured width, trailing padding) + `no_footer_row_ever_prints_half_a_hint_or_touches_the_location` (no half-hint, no touching) | pins |
| P-247 | `slash_model_direct_applies_instantly` (exact id applies, memory updated, `/bogus` does not start a run) + `model_partial_opens_the_picker_prefilled` (ambiguous family prefills, not switches) + `ctrl_l_opens_the_model_picker` | pins |
| P-290 | `timeline_picker_opens_and_enter_jumps` — `/timeline` opens instead of notice-dumping, Enter dismisses | pins |

### DSH Desktop (`parity/dsh-desktop.md`, 26 done, 5 sampled)

| Row | Test named | Verdict |
|---|---|---|
| D-084 | `goal_clear_clears_with_no_dialog_and_the_band_goes_with_it` — create, clear, cache empty, band gone from the frame | pins |
| S-101 | `control_slash_toggles_the_shortcuts_reference`, `escape_closes_the_shortcuts_reference`, `shortcut_rows_are_named_for_screen_readers` — the a11y one reads the node name through the real draw | pins |
| S-123 | `the_recorder_refuses_a_chord_another_command_owns_naming_both` — round 108's own test (this sample was drawn *after* it landed) | pins |
| L-114 | `new_session_button_names_its_chord` — the accessible name carries the effective chord **and** the drawn button shows it | pins |
| I-107 | `dropped_ask_denies_fail_closed` — spawns an ask, drops the responder, asserts the resolution is `Deny` | pins |

## Outcome

**17 rows sampled, 27 tests read, 0 downgraded.** The rows' `verified` cells are honest in this
sample — each named test drives the behaviour the row claims, through the draw path where the
behaviour is visual, with controls named where the row says so.

**What this does and does not prove.** The sample is 17 of 87 `done` rows (12 of 61 Pi, 5 of 26
DSH), every fifth by table order. It is evidence about the sample, not about the population:
the review's F5 finding was real when made and was closed by the work it named, and this sample
says the same class of defect is not currently visible in it. The check that would make this
continuous rather than periodic — reading each `verified` cell against its test automatically —
needs a judgement, not a script; until it exists, the sampling is the honest instrument and is
recorded here so the next one starts at row 21 rather than row 1.

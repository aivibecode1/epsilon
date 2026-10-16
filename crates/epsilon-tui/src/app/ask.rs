//! The model-question overlay: answering a parked `question` tool call.
//!
//! When the model asks the operator something, the turn parks on the shared
//! question hub and this overlay opens (seated from `draw`, next frame). One
//! question shows at a time with its options; `Tab` walks through several,
//! then a review stop submits them all — the opencode question-tab shape
//! (`references/opencode-1.18.34/packages/tui/src/routes/session/question.tsx`)
//! cut down to the think-picker chrome (`Panel`, `list_row`, `hint_row`).
//!
//! Answering rules, all fail-closed:
//! - Single-select: `Enter` answers and advances; the last one submits.
//! - Multi-select: `Enter`/`Space` toggles and stays; `Tab` advances; the
//!   review stop submits. Submitting with any question unanswered dismisses
//!   the whole call instead of inventing a default.
//! - `Esc` dismisses (closing custom input first). Dismissal resolves the
//!   parked turn to a `dismissed` ToolFailure, never to an answer.
//!
//! The overlay state lives on `App`, one slot per application instance — never
//! process-global. A global slot lets one test's parked question appear in
//! another test's frame (and eat its keys) when the suite runs threaded, which
//! is exactly the round9 battery failure this fixes; in the product there is
//! one `App` per process, so per-app state changes no live behaviour. The hub
//! stays shared (composition owns it); only the *seated overlay* is per-app.

use super::*;
use epsilon_contracts::question::QuestionAnswer;

/// Mirrors the tool's free-text cap so the overlay cannot stage what the
/// tool would refuse (`epsilon-tools/src/question.rs::MAX_CUSTOM_ANSWER_CHARS`).
const MAX_CUSTOM_CHARS: usize = 2000;

/// What the operator staged for one prompt: labels, or typed text.
type Staged = Vec<String>;

/// The open overlay: which parked question, where the cursor is, what is staged.
pub(crate) struct AskOverlay {
    pending: epsilon_runtime::PendingQuestion,
    q_index: usize,
    row: usize,
    staged: Vec<Staged>,
    custom_open: bool,
    custom_text: String,
    custom_cursor: usize,
}

#[derive(Default)]
pub(crate) struct AskSlot {
    pub(crate) overlay: Option<AskOverlay>,
    /// Id of the ask the last frame actually painted (the approval dialog's
    /// `approval_painted_id` shape): keys answer only what a frame showed.
    pub(crate) painted: Option<String>,
}

impl AskSlot {
    /// Whether an ask overlay is open (owns the keyboard).
    pub(crate) fn is_open(&self) -> bool {
        self.overlay.is_some()
    }
}

/// Seat any parked question and arm operator presence. Called from `draw`,
/// so the overlay opens on the next frame after the turn parks.
pub(crate) fn poll_ask(app: &mut App) {
    epsilon_runtime::shared_question_hub().set_operator_present(true);
    if app.ask_slot.overlay.is_some() {
        return;
    }
    if let Some(pending) = epsilon_runtime::shared_question_hub().take_pending() {
        app.ask_slot.overlay = Some(AskOverlay::new(pending));
    }
}

/// Forget which ask a frame painted (the too-small fallback: keys must not
/// answer what no frame showed).
pub(crate) fn clear_painted(app: &mut App) {
    app.ask_slot.painted = None;
}

/// Record which ask this frame painted.
pub(crate) fn note_painted(app: &mut App, id: String) {
    app.ask_slot.painted = Some(id);
}

/// Seat a parked question directly (tests: bypasses the hub drain).
#[cfg(test)]
pub(crate) fn seat_for_test(app: &mut App, pending: epsilon_runtime::PendingQuestion) {
    app.ask_slot.overlay = Some(AskOverlay::new(pending));
    app.ask_slot.painted = app.ask_slot.overlay.as_ref().map(|o| o.pending.id.clone());
}

/// Clear the slot (tests).
#[cfg(test)]
pub(crate) fn clear_for_test(app: &mut App) {
    app.ask_slot.overlay = None;
    app.ask_slot.painted = None;
}

impl AskOverlay {
    /// Whether the free-text field owns printable keys (footer hint switch).
    pub(crate) fn is_custom_open(&self) -> bool {
        self.custom_open
    }

    /// Whether the review stop is showing (footer hint switch).
    pub(crate) fn is_at_review(&self) -> bool {
        self.at_review()
    }

    fn new(pending: epsilon_runtime::PendingQuestion) -> Self {
        let staged = vec![Vec::new(); pending.prompts.len()];
        Self {
            pending,
            q_index: 0,
            row: 0,
            staged,
            custom_open: false,
            custom_text: String::new(),
            custom_cursor: 0,
        }
    }

    /// Stops: one per question, plus a review stop unless this is a single
    /// single-select (which submits on `Enter`, opencode `single()` parity).
    fn stops(&self) -> usize {
        let single = self.pending.prompts.len() == 1 && !self.pending.prompts[0].multiple;
        self.pending.prompts.len() + usize::from(!single)
    }

    fn at_review(&self) -> bool {
        self.stops() > self.pending.prompts.len() && self.q_index == self.pending.prompts.len()
    }

    /// Selectable rows on the current stop: options plus the custom row, or
    /// the lone review row.
    fn rows(&self) -> usize {
        if self.at_review() {
            return 1;
        }
        let prompt = &self.pending.prompts[self.q_index];
        prompt.options.len() + usize::from(prompt.custom)
    }

    fn on_custom_row(&self) -> bool {
        if self.at_review() {
            return false;
        }
        let prompt = &self.pending.prompts[self.q_index];
        prompt.custom && self.row == prompt.options.len()
    }

    fn move_row(&mut self, delta: i32) {
        let total = self.rows();
        if total == 0 {
            return;
        }
        let next = (self.row as i32 + delta).rem_euclid(total as i32);
        self.row = next as usize;
    }

    fn goto_stop(&mut self, index: usize) {
        let stops = self.stops();
        if stops == 0 {
            return;
        }
        self.q_index = index % stops;
        self.row = 0;
        self.custom_open = false;
        self.custom_text.clear();
        self.custom_cursor = 0;
    }

    fn insert_custom_char(&mut self, c: char) -> bool {
        if self.custom_text.chars().count() >= MAX_CUSTOM_CHARS {
            return false;
        }
        let byte = self
            .custom_text
            .char_indices()
            .nth(self.custom_cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.custom_text.len());
        self.custom_text.insert(byte, c);
        self.custom_cursor += 1;
        true
    }

    fn backspace_custom(&mut self) {
        if self.custom_cursor == 0 {
            return;
        }
        self.custom_cursor -= 1;
        let byte = self
            .custom_text
            .char_indices()
            .nth(self.custom_cursor)
            .map(|(i, _)| i)
            .unwrap_or(self.custom_text.len());
        self.custom_text.remove(byte);
    }
}

/// What one key did to the parked turn.
enum KeyOutcome {
    /// Still open; keep the overlay.
    Open,
    /// Answered everything: reply with one answer per prompt.
    Submit(Vec<QuestionAnswer>),
    /// Dismissed: resolve to failure, never an answer.
    Dismiss,
}

/// Keys while the ask overlay is open. Owns the keyboard like every other
/// overlay; never quits (the exit binding bypasses this branch so quitting
/// drops the parked ask, which fails closed).
pub(crate) fn handle_ask_key(app: &mut App, key: KeyEvent) -> bool {
    // The painted-id guard, mirroring the approval dialog: a key arriving
    // after a NEW ask seated and before the next draw must not answer what
    // no frame has shown.
    let guarded = match &app.ask_slot.overlay {
        Some(o) => app.ask_slot.painted.as_deref() == Some(o.pending.id.as_str()),
        None => false,
    };
    if !guarded {
        app.ui.push_notice(
            "the question is not visible yet — it appears on the next frame; \
             enlarge the terminal if it does not"
                .to_string(),
        );
        return false;
    }
    let outcome = {
        let slot = &mut app.ask_slot;
        let Some(o) = slot.overlay.as_mut() else {
            return false;
        };
        // Custom input owns printable keys; the list owns digits-as-jumps.
        if o.custom_open {
            match key.code {
                KeyCode::Esc => {
                    o.custom_open = false;
                    o.custom_text.clear();
                    o.custom_cursor = 0;
                    KeyOutcome::Open
                }
                KeyCode::Enter => {
                    let text = o.custom_text.trim().to_string();
                    o.custom_open = false;
                    o.custom_text.clear();
                    o.custom_cursor = 0;
                    if text.is_empty() {
                        return false;
                    }
                    commit_text(o, text)
                }
                KeyCode::Backspace => {
                    o.backspace_custom();
                    KeyOutcome::Open
                }
                KeyCode::Left => {
                    o.custom_cursor = o.custom_cursor.saturating_sub(1);
                    KeyOutcome::Open
                }
                KeyCode::Right => {
                    o.custom_cursor = (o.custom_cursor + 1).min(o.custom_text.chars().count());
                    KeyOutcome::Open
                }
                KeyCode::Char(c)
                    if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
                {
                    if !o.insert_custom_char(c) {
                        return false;
                    }
                    KeyOutcome::Open
                }
                _ => KeyOutcome::Open,
            }
        } else {
            match key.code {
                KeyCode::Esc => KeyOutcome::Dismiss,
                KeyCode::Up => {
                    o.move_row(-1);
                    KeyOutcome::Open
                }
                KeyCode::Down => {
                    o.move_row(1);
                    KeyOutcome::Open
                }
                KeyCode::Left => {
                    o.goto_stop(o.q_index + o.stops().saturating_sub(1));
                    KeyOutcome::Open
                }
                KeyCode::Right => {
                    o.goto_stop(o.q_index + 1);
                    KeyOutcome::Open
                }
                KeyCode::Tab => {
                    if key.modifiers == KeyModifiers::SHIFT {
                        o.goto_stop(o.q_index + o.stops().saturating_sub(1));
                    } else {
                        o.goto_stop(o.q_index + 1);
                    }
                    KeyOutcome::Open
                }
                KeyCode::Enter | KeyCode::Char(' ') => choose_current(o),
                KeyCode::Char(c)
                    if c.is_ascii_digit()
                        && (key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT) =>
                {
                    let want = (c as usize).saturating_sub('0' as usize).saturating_sub(1);
                    if want < o.rows() {
                        o.row = want;
                        choose_current(o)
                    } else {
                        KeyOutcome::Open
                    }
                }
                _ => KeyOutcome::Open,
            }
        }
    };
    match outcome {
        KeyOutcome::Submit(answers) => {
            if let Some(o) = app.ask_slot.overlay.take() {
                app.ask_slot.painted = None;
                o.pending.answer(answers);
            }
        }
        KeyOutcome::Dismiss => {
            if let Some(o) = app.ask_slot.overlay.take() {
                app.ask_slot.painted = None;
                o.pending.dismiss();
                app.ui.push_notice(
                    "question dismissed — the turn proceeds without the operator's answer",
                );
            }
        }
        _ => {}
    }
    false
}

/// `Enter`/`Space`/digit on the current row.
fn choose_current(o: &mut AskOverlay) -> KeyOutcome {
    if o.at_review() {
        // An empty selection is a dismissal, not a default: any question
        // left unanswered dismisses the whole call.
        if o.staged.iter().any(|a| a.is_empty()) {
            return KeyOutcome::Dismiss;
        }
        return KeyOutcome::Submit(o.staged.clone());
    }
    if o.on_custom_row() {
        o.custom_open = true;
        o.custom_text.clear();
        o.custom_cursor = 0;
        return KeyOutcome::Open;
    }
    let Some(label) = o.pending.prompts[o.q_index]
        .options
        .get(o.row)
        .map(|opt| opt.label.clone())
    else {
        return KeyOutcome::Open;
    };
    if o.pending.prompts[o.q_index].multiple {
        let staged = &mut o.staged[o.q_index];
        if let Some(pos) = staged.iter().position(|s| s == &label) {
            staged.remove(pos);
        } else {
            staged.push(label);
        }
        KeyOutcome::Open
    } else {
        o.staged[o.q_index] = vec![label];
        advance(o)
    }
}

/// A committed custom line: staged always; advancing only for single-select.
fn commit_text(o: &mut AskOverlay, text: String) -> KeyOutcome {
    if o.at_review() {
        return KeyOutcome::Open;
    }
    if o.pending.prompts[o.q_index].multiple {
        let staged = &mut o.staged[o.q_index];
        if let Some(pos) = staged.iter().position(|s| s == &text) {
            staged.remove(pos);
        } else {
            staged.push(text);
        }
        KeyOutcome::Open
    } else {
        o.staged[o.q_index] = vec![text];
        advance(o)
    }
}

/// Move to the next stop, or submit when past the end.
fn advance(o: &mut AskOverlay) -> KeyOutcome {
    if o.q_index + 1 >= o.stops() {
        if o.staged.iter().any(|a| a.is_empty()) {
            return KeyOutcome::Dismiss;
        }
        KeyOutcome::Submit(o.staged.clone())
    } else {
        o.goto_stop(o.q_index + 1);
        KeyOutcome::Open
    }
}

/// Owned draw snapshot, built under the lock so rendering holds none.
pub(crate) struct AskFrame {
    pub(crate) id: String,
    pub(crate) show_tabs: bool,
    pub(crate) tabs: Vec<(String, bool, bool)>,
    pub(crate) question: String,
    pub(crate) multi: bool,
    pub(crate) rows: Vec<AskFrameRow>,
    pub(crate) selected: usize,
    pub(crate) custom_open: bool,
    pub(crate) custom_text: String,
    pub(crate) at_review: bool,
    pub(crate) review: Vec<(String, String)>,
}

/// One rendered row: label, `picked` mark when staged, dim description after.
pub(crate) struct AskFrameRow {
    pub(crate) label: String,
    pub(crate) picked: bool,
    pub(crate) description: String,
}

fn clean(text: &str) -> String {
    epsilon_runtime::text_safety::display_safe(text)
        .0
        .into_owned()
}

/// Snapshot the open overlay for the draw path. `None` when closed.
pub(crate) fn draw_snapshot(app: &App) -> Option<AskFrame> {
    let o = app.ask_slot.overlay.as_ref()?;
    let id = o.pending.id.clone();
    let show_tabs = o.pending.prompts.len() > 1;
    let tabs = o
        .pending
        .prompts
        .iter()
        .enumerate()
        .map(|(i, p)| (clean(&p.header), i == o.q_index, !o.staged[i].is_empty()))
        .collect();
    if o.at_review() {
        let review = o
            .pending
            .prompts
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let value = if o.staged[i].is_empty() {
                    "(not answered)".to_string()
                } else {
                    o.staged[i].join(", ")
                };
                (clean(&p.header), clean(&value))
            })
            .collect();
        return Some(AskFrame {
            id,
            show_tabs,
            tabs,
            question: "Review".into(),
            multi: false,
            rows: vec![AskFrameRow {
                label: "Submit answers".into(),
                picked: false,
                description: "unanswered questions dismiss the call instead".into(),
            }],
            selected: 0,
            custom_open: false,
            custom_text: String::new(),
            at_review: true,
            review,
        });
    }
    let prompt = &o.pending.prompts[o.q_index];
    let mut rows: Vec<AskFrameRow> = prompt
        .options
        .iter()
        .map(|opt| AskFrameRow {
            label: clean(&opt.label),
            picked: o.staged[o.q_index].contains(&opt.label),
            description: clean(&opt.description),
        })
        .collect();
    if prompt.custom {
        rows.push(AskFrameRow {
            label: "Type your own answer".into(),
            picked: o.staged[o.q_index]
                .iter()
                .any(|s| !prompt.options.iter().any(|opt| &opt.label == s)),
            description: String::new(),
        });
    }
    Some(AskFrame {
        id,
        show_tabs,
        tabs,
        question: clean(&prompt.question),
        multi: prompt.multiple,
        rows,
        selected: o.row,
        custom_open: o.custom_open,
        custom_text: clean(&o.custom_text),
        at_review: false,
        review: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::tests_support::{render_rows, test_app};
    use super::*;
    use crossterm::event::KeyEventState;
    use epsilon_contracts::question::{QuestionOption, QuestionPrompt};

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent {
            code,
            modifiers: KeyModifiers::NONE,
            kind: KeyEventKind::Press,
            state: KeyEventState::NONE,
        }
    }

    fn option(label: &str) -> QuestionOption {
        QuestionOption {
            label: label.into(),
            description: format!("go {label}"),
        }
    }

    fn two_prompts() -> Vec<QuestionPrompt> {
        vec![
            QuestionPrompt {
                question: "Which side?".into(),
                header: "side".into(),
                options: vec![option("up"), option("down")],
                multiple: false,
                custom: false,
            },
            QuestionPrompt {
                question: "How fast?".into(),
                header: "pace".into(),
                options: vec![option("fast"), option("slow")],
                multiple: true,
                custom: true,
            },
        ]
    }

    /// Park on a local hub and seat the pending question in this app, returning
    /// the waiter's handle. Local hub and per-app slot: parallel tests never
    /// touch each other's overlays *or* pendings.
    async fn park_and_seat(
        app: &mut App,
        prompts: Vec<QuestionPrompt>,
    ) -> (
        std::sync::Arc<epsilon_runtime::QuestionHub>,
        tokio::task::JoinHandle<
            Result<Vec<QuestionAnswer>, epsilon_contracts::question::QuestionError>,
        >,
    ) {
        let hub = std::sync::Arc::new(epsilon_runtime::QuestionHub::new());
        hub.set_operator_present(true);
        let waiter = tokio::spawn({
            let hub = std::sync::Arc::clone(&hub);
            async move {
                use epsilon_contracts::question::QuestionService;
                hub.ask("s".into(), prompts).await
            }
        });
        while hub.pending_count() == 0 {
            tokio::task::yield_now().await;
        }
        seat_for_test(app, hub.take_pending().expect("the parked question"));
        (hub, waiter)
    }

    async fn waiter_answers(
        waiter: tokio::task::JoinHandle<
            Result<Vec<QuestionAnswer>, epsilon_contracts::question::QuestionError>,
        >,
    ) -> Vec<QuestionAnswer> {
        tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("answering must settle the parked turn, never hang")
            .expect("the waiter task stays alive")
            .expect("the answer settles")
    }

    #[tokio::test]
    async fn parked_question_renders_its_options_enter_answers_esc_dismisses() {
        let (mut app, agent, _rx) = test_app();
        // Pin the repertoire: the suite names glyphs, never assumes them.
        app.theme.glyphs = crate::ui::theme::UNICODE;
        let (_hub, waiter) = park_and_seat(&mut app, two_prompts()).await;

        // The frame, not the state: options, tab headers, and the custom row
        // must reach the screen the operator reads.
        let screen = render_rows(&mut app, 80, 24).join("\n");
        assert!(
            screen.contains("Which side?"),
            "question renders:\n{screen}"
        );
        assert!(screen.contains("up"), "options render:\n{screen}");
        assert!(screen.contains("down"), "options render:\n{screen}");
        assert!(
            screen.contains("side") && screen.contains("pace"),
            "tab headers render:\n{screen}"
        );

        // Enter answers the highlighted single-select and advances.
        assert!(!super::super::keys::handle_key(
            &mut app,
            &agent,
            key(KeyCode::Enter)
        ));
        assert!(
            app.ask_slot.is_open(),
            "a second question keeps the overlay open"
        );
        let screen = render_rows(&mut app, 80, 24).join("\n");
        assert!(
            screen.contains("How fast?"),
            "advances to the next question:\n{screen}"
        );
        assert!(
            screen.contains("Type your own answer"),
            "the custom row renders:\n{screen}"
        );

        // Esc dismisses the whole call: a failure, never a partial answer.
        assert!(!super::super::keys::handle_key(
            &mut app,
            &agent,
            key(KeyCode::Esc)
        ));
        assert!(!app.ask_slot.is_open(), "dismissal closes the overlay");
        let settled = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("dismissal must settle the parked turn, never hang")
            .expect("the waiter task stays alive");
        assert!(
            matches!(
                settled,
                Err(epsilon_contracts::question::QuestionError::Dismissed)
            ),
            "dismissal resolves to dismissed: {settled:?}"
        );
        clear_for_test(&mut app);
    }

    /// While the overlay owns the keyboard, the footer names its keys — not
    /// the composer keys, which would not work (the ADR-044 wart). The frame,
    /// not the state: the last row must read as a question footer.
    #[tokio::test]
    async fn open_question_names_its_keys_in_the_footer() {
        let (mut app, _agent, _rx) = test_app();
        app.theme.glyphs = crate::ui::theme::UNICODE;
        let (_hub, _waiter) = park_and_seat(&mut app, two_prompts()).await;

        let rows = render_rows(&mut app, 80, 24);
        let footer = rows.last().cloned().unwrap_or_default();
        assert!(
            footer.contains("question"),
            "the footer names the owner of the keyboard:\n{}",
            rows.join("\n")
        );
        assert!(
            footer.contains("dismiss"),
            "Esc dismisses, and the footer says so:\n{}",
            rows.join("\n")
        );
        assert!(
            !footer.contains("insert"),
            "composer keys must not be advertised while the overlay owns them:\n{}",
            rows.join("\n")
        );
        clear_for_test(&mut app);
    }

    #[tokio::test]
    async fn multiple_toggle_custom_review_submit_round_trip() {
        let (mut app, agent, _rx) = test_app();
        app.theme.glyphs = crate::ui::theme::UNICODE;
        let (_hub, waiter) = park_and_seat(&mut app, two_prompts()).await;

        // Answer the single-select, landing on the multi-select.
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Enter));
        // Space toggles "fast" and stays; arrows move.
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Char(' ')));
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Down));
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Down));
        // On the custom row: Enter opens input, typing + Enter stages it.
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Enter));
        for c in "brisk".chars() {
            super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Char(c)));
        }
        let screen = render_rows(&mut app, 80, 24).join("\n");
        assert!(
            screen.contains("brisk"),
            "typed custom text reaches the screen:\n{screen}"
        );
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Enter));
        // Tab to the review stop and submit.
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Tab));
        let screen = render_rows(&mut app, 80, 24).join("\n");
        assert!(screen.contains("Review"), "review stop renders:\n{screen}");
        assert!(
            screen.contains("fast, brisk"),
            "staged answers render for review:\n{screen}"
        );
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Enter));

        assert!(!app.ask_slot.is_open(), "submit closes the overlay");
        assert_eq!(
            waiter_answers(waiter).await,
            vec![
                vec!["up".to_string()],
                vec!["fast".to_string(), "brisk".to_string()]
            ],
            "labels round-trip in prompt order"
        );
        clear_for_test(&mut app);
    }

    #[tokio::test]
    async fn submitting_with_a_question_unanswered_dismisses() {
        let (mut app, agent, _rx) = test_app();
        app.theme.glyphs = crate::ui::theme::UNICODE;
        let (_hub, waiter) = park_and_seat(
            &mut app,
            vec![QuestionPrompt {
                question: "How fast?".into(),
                header: "pace".into(),
                options: vec![option("fast"), option("slow")],
                multiple: true,
                custom: true,
            }],
        )
        .await;

        // Tab straight to review with nothing staged, then submit: an empty
        // selection is a dismissal, not a default.
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Tab));
        super::super::keys::handle_key(&mut app, &agent, key(KeyCode::Enter));
        assert!(!app.ask_slot.is_open());
        let settled = tokio::time::timeout(std::time::Duration::from_secs(5), waiter)
            .await
            .expect("must settle, never hang")
            .expect("the waiter task stays alive");
        assert!(
            matches!(
                settled,
                Err(epsilon_contracts::question::QuestionError::Dismissed)
            ),
            "empty multi-select dismisses: {settled:?}"
        );
        clear_for_test(&mut app);
    }
}

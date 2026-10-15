//! The question tool: model-initiated clarifying questions to the operator.
//!
//! Criterion 4. The model calls `question` when it needs a preference, a
//! disambiguation, or a direction choice; the call BLOCKS the turn until the
//! operator answers (Sequential, like `task`), and the answers return as
//! labels the model continues from.
//!
//! Shape adapted from opencode (`references/opencode-1.18.34`: the tool in
//! `packages/core/src/tool/question.ts`, the service in
//! `packages/core/src/question.ts`, the `Option`/`Prompt`/`Answer` vocabulary
//! in `packages/schema/src/question.ts`) and Claude Code's AskUserQuestion
//! surface (options with labels + descriptions, a multiple-select flag, a
//! free-text answer, recommended-first convention in the description).
//!
//! Fail-closed rules (see the ADR amendment):
//! - No operator attached (headless/print/RPC/serve) resolves IMMEDIATELY to
//!   a ToolFailure telling the model to proceed with its best judgment. A
//!   hung turn is worse than a refused one.
//! - Dismissal (Esc/reject, or an empty multi-select) resolves to a
//!   ToolFailure ("dismissed"), never a fabricated answer.
//! - A prompt with no options and no custom input is unanswerable — refused
//!   at parse, not at render.
//!
//! The tool speaks only [`QuestionService`][epsilon_contracts::question::QuestionService]:
//! parking and answering live with the provisioned service, so this crate
//! needs no edge to any frontend.

use std::sync::Arc;

use async_trait::async_trait;
use epsilon_contracts::question::{QuestionError, QuestionOption, QuestionPrompt, QuestionService};
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Caps so one pathological call cannot flood the context or the screen.
///
/// Documented in the ADR amendment: at most 4 questions per call, at most 6
/// options per question; text lengths bounded so one call cannot smuggle a
/// transcript's worth of prose through labels.
pub const MAX_QUESTIONS: usize = 4;
/// Maximum options on one prompt.
pub const MAX_OPTIONS_PER_QUESTION: usize = 6;
/// Maximum characters of the question text itself.
pub const MAX_QUESTION_CHARS: usize = 500;
/// Maximum characters of the tab header (opencode: "max 30 chars").
pub const MAX_HEADER_CHARS: usize = 30;
/// Maximum characters of one option label.
pub const MAX_LABEL_CHARS: usize = 80;
/// Maximum characters of one option description.
pub const MAX_OPTION_DESCRIPTION_CHARS: usize = 300;
/// Maximum characters of a typed free-text answer.
pub const MAX_CUSTOM_ANSWER_CHARS: usize = 2000;

/// Model-facing description, kept here (not in `descriptions/`) so the tool
/// stays one file: wording, schema, validation, and tests review together.
pub const QUESTION_DESC: &str = "Use this tool when you need to ask the operator questions during execution. This allows you to:\n1. Gather preferences or requirements\n2. Clarify ambiguous instructions\n3. Get decisions on implementation choices as you work\n4. Offer choices to the operator about what direction to take.\n\nUsage notes:\n- When `custom` is enabled (default), a \"Type your own answer\" option is added automatically; don't include \"Other\" or catch-all options\n- Answers are returned as arrays of labels; set `multiple: true` to allow selecting more than one\n- If you recommend a specific option, make that the first option in the list and add \"(Recommended)\" at the end of the label\n- The call blocks until the operator answers; where no operator is attached it fails immediately — proceed with your best judgment then\n- At most 4 questions per call, at most 6 options per question; a question with no options must leave `custom` enabled";

/// One labelled choice, as the model sends it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QuestionOptionArgs {
    /// Display text (1-5 words, concise). Answers come back as labels.
    pub label: String,
    /// Explanation of the choice.
    #[serde(default)]
    pub description: String,
}

/// One question, as the model sends it.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QuestionPromptArgs {
    /// The complete question.
    pub question: String,
    /// Very short label (max 30 chars). Empty derives from the question.
    #[serde(default)]
    pub header: String,
    /// Available choices. May be empty only when `custom` is not false.
    #[serde(default)]
    pub options: Vec<QuestionOptionArgs>,
    /// Allow selecting more than one choice.
    #[serde(default)]
    pub multiple: bool,
    /// Allow typing a free-text answer (default true).
    #[serde(default)]
    pub custom: Option<bool>,
}

/// Arguments for the `question` tool.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct QuestionArgs {
    /// Questions to ask (1-4).
    pub questions: Vec<QuestionPromptArgs>,
}

/// Model-invoked clarifying questions to the operator.
pub struct QuestionTool {
    descriptor: ToolDescriptor,
    service: Arc<dyn QuestionService>,
}

impl QuestionTool {
    /// Build a `question` tool over the provisioned operator service.
    pub fn new(service: Arc<dyn QuestionService>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "question".into(),
                label: "Question".into(),
                description: QUESTION_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(QuestionArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                // A question parks the turn until a human answers — barrier
                // semantics like `task`, never interleaved with parallel calls.
                execution_mode: ExecutionMode::Sequential,
            },
            service,
        }
    }
}

#[async_trait]
impl Tool for QuestionTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        ctx: ToolContext,
        on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        if ctx.token.is_cancelled() {
            return Err(ToolFailure::msg(
                "[aborted] question cancelled before start",
            ));
        }

        let args: QuestionArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'question': {e}")))?;
        let prompts = validate(args)?;

        // The screen the operator will see is one frame away: say so on the
        // transcript's live card, so a parked turn reads as waiting, not hung.
        on_update(epsilon_contracts::tool::ToolPartial {
            content: vec![epsilon_contracts::content::ContentBlock::text(
                "question awaiting the operator…",
            )],
            details: serde_json::json!({"questions": prompts.len()}),
        });

        let answers = tokio::select! {
            settled = self.service.ask(ctx.session_id.clone(), prompts.clone()) => settled,
            () = ctx.token.cancelled() => {
                return Err(ToolFailure::msg(
                    "[aborted] question cancelled while awaiting the operator",
                ));
            }
        };
        match answers {
            Ok(answers) => {
                let text = epsilon_contracts::question::format_model_output(&prompts, &answers);
                Ok(ToolOutput {
                    content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                    details: serde_json::json!({
                        "answers": answers,
                        "questions": prompts
                            .iter()
                            .map(|p| p.question.clone())
                            .collect::<Vec<_>>(),
                    }),
                    usage: None,
                    terminate: false,
                })
            }
            Err(QuestionError::NoOperator(detail)) => Err(ToolFailure::msg(format!(
                "question: {detail} — no operator is attached to this run, so the \
                 question cannot be answered. Proceed with your best judgment and \
                 say what you assumed."
            ))),
            Err(QuestionError::Dismissed) => Err(ToolFailure::msg(
                "question: dismissed — the operator dismissed the question without \
                 answering. Proceed with your best judgment and say what you assumed.",
            )),
            Err(QuestionError::SessionEnded(detail)) => Err(ToolFailure::msg(format!(
                "question: {detail} — the session ended before the operator answered. \
                 Proceed with your best judgment and say what you assumed."
            ))),
        }
    }
}

/// Validate model arguments into service prompts, refusing at parse what no
/// operator could answer or what would flood the context.
fn validate(args: QuestionArgs) -> Result<Vec<QuestionPrompt>, ToolFailure> {
    if args.questions.is_empty() {
        return Err(ToolFailure::msg(
            "question: at least one question is required",
        ));
    }
    if args.questions.len() > MAX_QUESTIONS {
        return Err(ToolFailure::msg(format!(
            "question: at most {MAX_QUESTIONS} questions per call, got {} — split the rest into a second call",
            args.questions.len()
        )));
    }
    args.questions
        .into_iter()
        .enumerate()
        .map(|(index, q)| validate_prompt(index, q))
        .collect()
}

/// Validate one prompt: non-empty text, sane counts, and answerable.
fn validate_prompt(index: usize, q: QuestionPromptArgs) -> Result<QuestionPrompt, ToolFailure> {
    let tag = format!("question: question {}:", index + 1);
    let question = q.question.trim().to_string();
    if question.is_empty() {
        return Err(ToolFailure::msg(format!(
            "{tag} the question text is required"
        )));
    }
    if question.chars().count() > MAX_QUESTION_CHARS {
        return Err(ToolFailure::msg(format!(
            "{tag} the question must stay under {MAX_QUESTION_CHARS} characters"
        )));
    }
    if q.options.len() > MAX_OPTIONS_PER_QUESTION {
        return Err(ToolFailure::msg(format!(
            "{tag} at most {MAX_OPTIONS_PER_QUESTION} options per question, got {}",
            q.options.len()
        )));
    }
    let custom = q.custom.unwrap_or(true);
    if q.options.is_empty() && !custom {
        return Err(ToolFailure::msg(format!(
            "{tag} unanswerable: no options and custom input disabled — provide at \
             least one option or leave `custom` enabled"
        )));
    }
    let options = q
        .options
        .into_iter()
        .enumerate()
        .map(|(oi, o)| {
            let label = o.label.trim().to_string();
            if label.is_empty() {
                return Err(ToolFailure::msg(format!(
                    "{tag} option {} has an empty label",
                    oi + 1
                )));
            }
            if label.chars().count() > MAX_LABEL_CHARS {
                return Err(ToolFailure::msg(format!(
                    "{tag} option {} must stay under {MAX_LABEL_CHARS} characters",
                    oi + 1
                )));
            }
            if o.description.chars().count() > MAX_OPTION_DESCRIPTION_CHARS {
                return Err(ToolFailure::msg(format!(
                    "{tag} option {} description must stay under {MAX_OPTION_DESCRIPTION_CHARS} characters",
                    oi + 1
                )));
            }
            Ok(QuestionOption {
                label,
                description: o.description,
            })
        })
        .collect::<Result<Vec<_>, ToolFailure>>()?;
    // An empty header derives from the question, so the tab row names itself
    // without refusing a call over chrome.
    let header = {
        let h = q.header.trim();
        if h.is_empty() {
            question.chars().take(MAX_HEADER_CHARS).collect()
        } else {
            h.to_string()
        }
    };
    if header.chars().count() > MAX_HEADER_CHARS {
        return Err(ToolFailure::msg(format!(
            "{tag} the header must stay under {MAX_HEADER_CHARS} characters"
        )));
    }
    Ok(QuestionPrompt {
        question,
        header,
        options,
        multiple: q.multiple,
        custom,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::question::QuestionAnswer;
    use std::sync::Mutex;

    /// Scripted operator: answers, dismisses, or is absent on demand.
    enum StubMode {
        Answer(Vec<QuestionAnswer>),
        Dismiss,
        NoOperator,
    }

    struct Stub {
        mode: StubMode,
        asked: Mutex<bool>,
    }

    impl Stub {
        fn answering(answers: Vec<QuestionAnswer>) -> Self {
            Self {
                mode: StubMode::Answer(answers),
                asked: Mutex::new(false),
            }
        }

        fn dismissing() -> Self {
            Self {
                mode: StubMode::Dismiss,
                asked: Mutex::new(false),
            }
        }

        fn absent() -> Self {
            Self {
                mode: StubMode::NoOperator,
                asked: Mutex::new(false),
            }
        }
    }

    #[async_trait]
    impl QuestionService for Stub {
        async fn ask(
            &self,
            _session_id: String,
            _questions: Vec<QuestionPrompt>,
        ) -> Result<Vec<QuestionAnswer>, QuestionError> {
            *self.asked.lock().unwrap() = true;
            match &self.mode {
                StubMode::Answer(a) => Ok(a.clone()),
                StubMode::Dismiss => Err(QuestionError::Dismissed),
                StubMode::NoOperator => Err(QuestionError::NoOperator(
                    "no operator is attached to this session".into(),
                )),
            }
        }
    }

    fn ctx() -> ToolContext {
        ToolContext {
            session_id: "t".into(),
            observation_scope: Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir(),
            token: Default::default(),
            task_host: None,
            goal_host: None,
        }
    }

    async fn run(service: Arc<Stub>, args: serde_json::Value) -> Result<ToolOutput, ToolFailure> {
        QuestionTool::new(service)
            .execute(
                ToolInvocation {
                    tool_call_id: "tc-q-1".into(),
                    args,
                },
                ctx(),
                Arc::new(|_| {}),
            )
            .await
    }

    fn text_of(out: &ToolOutput) -> String {
        match &out.content[0] {
            epsilon_contracts::content::ContentBlock::Text { text } => text.clone(),
            _ => panic!("text expected"),
        }
    }

    fn one_question() -> serde_json::Value {
        serde_json::json!({
            "questions": [{
                "question": "Which side?",
                "header": "side",
                "options": [
                    {"label": "up", "description": "go up"},
                    {"label": "down", "description": "go down"}
                ]
            }]
        })
    }

    #[tokio::test]
    async fn no_operator_fails_closed_without_hanging() {
        let stub = Arc::new(Stub::absent());
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            run(Arc::clone(&stub), one_question()),
        )
        .await
        .expect("a question with no operator must resolve, never hang");
        let err = out.unwrap_err();
        assert!(
            err.message.contains("no operator"),
            "names the closed shape: {}",
            err.message
        );
        assert!(
            err.message.contains("best judgment"),
            "tells the model to proceed: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn dismissal_is_a_failure_never_a_fabricated_answer() {
        let stub = Arc::new(Stub::dismissing());
        let err = run(Arc::clone(&stub), one_question()).await.unwrap_err();
        assert!(
            err.message.contains("dismissed"),
            "dismissal resolves to a dismissed failure: {}",
            err.message
        );
    }

    #[tokio::test]
    async fn unanswerable_question_is_refused_at_parse() {
        // No options and no custom input: nobody could answer it, so the
        // service must never even be asked.
        let stub = Arc::new(Stub::answering(vec![vec!["up".into()]]));
        let err = run(
            Arc::clone(&stub),
            serde_json::json!({
                "questions": [{
                    "question": "Which side?",
                    "header": "side",
                    "options": [],
                    "custom": false
                }]
            }),
        )
        .await
        .unwrap_err();
        assert!(
            err.message.contains("unanswerable"),
            "refused at parse: {}",
            err.message
        );
        assert!(
            !*stub.asked.lock().unwrap(),
            "the operator is never asked an unanswerable question"
        );
    }

    #[tokio::test]
    async fn option_custom_multiple_round_trip() {
        let stub = Arc::new(Stub::answering(vec![
            vec!["up".into()],
            vec!["fast".into(), "typed words".into()],
        ]));
        let out = run(
            Arc::clone(&stub),
            serde_json::json!({
                "questions": [
                    {
                        "question": "Which side?",
                        "header": "side",
                        "options": [{"label": "up", "description": "go up"}]
                    },
                    {
                        "question": "How?",
                        "header": "how",
                        "multiple": true,
                        "options": [
                            {"label": "fast", "description": "be fast"},
                            {"label": "slow", "description": "be slow"}
                        ],
                        "custom": true
                    }
                ]
            }),
        )
        .await
        .unwrap();
        let text = text_of(&out);
        assert!(
            text.contains("\"Which side?\"=\"up\""),
            "single answer formats bare: {text}"
        );
        assert!(
            text.contains("\"How?\"=\"fast, typed words\""),
            "multiple + custom answers join: {text}"
        );
        assert!(
            text.contains("User has answered your questions"),
            "opencode toModelOutput shape: {text}"
        );
    }

    #[tokio::test]
    async fn empty_questions_and_over_caps_are_refused() {
        let stub = Arc::new(Stub::answering(vec![]));
        let err = run(Arc::clone(&stub), serde_json::json!({"questions": []}))
            .await
            .unwrap_err();
        assert!(err.message.contains("at least one question"), "{err:?}");

        let many: Vec<_> = (0..9)
            .map(|i| {
                serde_json::json!({
                    "question": format!("Q{i}?"),
                    "header": "h",
                    "options": [{"label": "a", "description": "b"}]
                })
            })
            .collect();
        let err = run(Arc::clone(&stub), serde_json::json!({"questions": many}))
            .await
            .unwrap_err();
        assert!(err.message.contains("at most 4 questions"), "{err:?}");
    }
}

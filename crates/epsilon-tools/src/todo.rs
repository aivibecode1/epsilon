//! The todo tool: session task-list replacement (opencode `tool/todo.ts`
//! semantics, simplified).
//!
//! The model sends the FULL list every call — there is no hidden state to
//! corrupt and no persistence layer to design; the transcript itself is the
//! record (the same reason pi ships no todo store). Statuses follow the
//! opencode ladder: pending → in_progress → completed.

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};
use schemars::JsonSchema;
use serde::Deserialize;

/// Hard caps so one pathological call cannot flood the context.
const MAX_ITEMS: usize = 50;
const MAX_CONTENT_CHARS: usize = 500;

/// Ladder state of one list entry.
#[derive(Debug, Clone, Copy, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    /// Not started.
    Pending,
    /// Currently being worked on (at most one is sensible; not enforced).
    InProgress,
    /// Finished.
    Completed,
}

impl TodoStatus {
    fn marker(self) -> &'static str {
        match self {
            Self::Pending => "[ ]",
            Self::InProgress => "[~]",
            Self::Completed => "[x]",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
        }
    }
}

/// One checklist entry.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TodoItemArgs {
    /// Short imperative description of the step.
    pub content: String,
    /// Where this step stands in the pending/in_progress/completed ladder.
    pub status: TodoStatus,
}

/// Arguments for the todo tool: the complete replacement list.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct TodoArgs {
    /// The full task list; replaces whatever the previous call sent.
    pub items: Vec<TodoItemArgs>,
}

pub const TODO_DESC: &str = include_str!("../descriptions/todo.txt");

/// Stateless checklist tracker: validates, echoes the list back, and publishes
/// it in `details` for UI rendering (details never reach provider prompts).
pub struct TodoTool {
    descriptor: ToolDescriptor,
}

impl TodoTool {
    /// Build the tool.
    pub fn new() -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: "todo".into(),
                label: "Todo".into(),
                description: TODO_DESC.into(),
                parameters_schema: serde_json::to_value(schemars::schema_for!(TodoArgs))
                    .unwrap_or(serde_json::json!({"type":"object"})),
                execution_mode: ExecutionMode::Parallel,
            },
        }
    }
}

impl Default for TodoTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for TodoTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        let args: TodoArgs = serde_json::from_value(call.args)
            .map_err(|e| ToolFailure::msg(format!("invalid arguments for 'todo': {e}")))?;
        if args.items.len() > MAX_ITEMS {
            return Err(ToolFailure::msg(format!(
                "todo lists are capped at {MAX_ITEMS} items; split the work"
            )));
        }
        for item in &args.items {
            if item.content.trim().is_empty() {
                return Err(ToolFailure::msg("todo items must have content"));
            }
            if item.content.chars().count() > MAX_CONTENT_CHARS {
                return Err(ToolFailure::msg(
                    "todo items must stay under 500 characters; keep steps short",
                ));
            }
        }

        let done = args
            .items
            .iter()
            .filter(|i| i.status == TodoStatus::Completed)
            .count();
        let mut body = args
            .items
            .iter()
            .map(|i| format!("{} {}", i.status.marker(), i.content.trim()))
            .collect::<Vec<_>>()
            .join("\n");
        if body.is_empty() {
            body = "(empty list — all tasks cleared)".to_string();
        }
        Ok(ToolOutput {
            content: vec![epsilon_contracts::content::ContentBlock::text(format!(
                "{body}\n({done}/{} completed — send the full list to update statuses)",
                args.items.len()
            ))],
            details: serde_json::json!({
                "todo": args.items.iter().map(|i| serde_json::json!({
                    "content": i.content.trim(),
                    "status": i.status.as_str(),
                })).collect::<Vec<_>>()
            }),
            usage: None,
            terminate: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use epsilon_contracts::content::ContentBlock;

    async fn run(args: serde_json::Value) -> Result<ToolOutput, ToolFailure> {
        let ctx = ToolContext {
            session_id: "t".into(),
            observation_scope: std::sync::Arc::new(()),
            cwd: std::env::temp_dir(),
            spill_dir: std::env::temp_dir(),
            token: Default::default(),
            task_host: None,
        };
        TodoTool::new()
            .execute(
                ToolInvocation {
                    tool_call_id: "tc".into(),
                    args,
                },
                ctx,
                Arc::new(|_| {}),
            )
            .await
    }

    use std::sync::Arc;

    #[tokio::test]
    async fn renders_checklist_with_counts_and_details() {
        let out = run(serde_json::json!({"items": [
            {"content": "read config", "status": "completed"},
            {"content": "fix bug", "status": "in_progress"},
            {"content": "add test", "status": "pending"}
        ]}))
        .await
        .unwrap();
        let text = match &out.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("text expected"),
        };
        assert!(text.contains("[x] read config"));
        assert!(text.contains("[~] fix bug"));
        assert!(text.contains("[ ] add test"));
        assert!(text.contains("(1/3 completed"));
        assert_eq!(out.details["todo"].as_array().unwrap().len(), 3);
    }

    #[tokio::test]
    async fn rejects_empty_content_and_oversized_lists() {
        let err = run(serde_json::json!({"items": [{"content": "  ", "status": "pending"}]}))
            .await
            .unwrap_err();
        assert!(err.message.contains("must have content"));

        let big: Vec<serde_json::Value> = (0..51)
            .map(|i| serde_json::json!({"content": format!("task {i}"), "status": "pending"}))
            .collect();
        let err = run(serde_json::json!({ "items": big })).await.unwrap_err();
        assert!(err.message.contains("capped at 50"));
    }

    #[tokio::test]
    async fn empty_list_is_a_valid_clear_signal() {
        let out = run(serde_json::json!({"items": []})).await.unwrap();
        let text = match &out.content[0] {
            ContentBlock::Text { text } => text.clone(),
            _ => panic!("text expected"),
        };
        assert!(text.contains("all tasks cleared"));
    }
}

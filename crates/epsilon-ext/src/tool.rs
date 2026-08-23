//! `ExtTool`: presents a manifest-declared extension tool as an ordinary registry tool.

use std::sync::Arc;

use async_trait::async_trait;
use epsilon_contracts::tool::{
    ExecutionMode, Tool, ToolContext, ToolDescriptor, ToolFailure, ToolInvocation, ToolOutput,
    ToolUpdateSink,
};

use crate::host::{ExtError, ExtHost};
use crate::manifest::ExtToolSpec;

/// Registry-facing wrapper over one declared extension tool.
pub struct ExtTool {
    descriptor: ToolDescriptor,
    host: Arc<ExtHost>,
}

impl ExtTool {
    /// Build from the manifest spec + live host. Extension tools are Sequential: the
    /// extension owns its own serialization.
    pub fn new(spec: &ExtToolSpec, host: Arc<ExtHost>) -> Self {
        Self {
            descriptor: ToolDescriptor {
                name: spec.name.clone(),
                label: format!("ext:{}", spec.name),
                description: spec.description.clone(),
                parameters_schema: spec.schema.clone(),
                execution_mode: ExecutionMode::Sequential,
            },
            host,
        }
    }
}

#[async_trait]
impl Tool for ExtTool {
    fn descriptor(&self) -> &ToolDescriptor {
        &self.descriptor
    }

    async fn execute(
        &self,
        call: ToolInvocation,
        _ctx: ToolContext,
        _on_update: ToolUpdateSink,
    ) -> Result<ToolOutput, ToolFailure> {
        match self
            .host
            .invoke(&self.descriptor.name, &call.tool_call_id, call.args)
            .await
        {
            Ok((text, mut details)) => {
                if let Some(obj) = details.as_object_mut() {
                    obj.insert(
                        "extension".into(),
                        serde_json::Value::String(self.host.name().into()),
                    );
                }
                Ok(ToolOutput {
                    content: vec![epsilon_contracts::content::ContentBlock::text(text)],
                    details,
                    usage: None,
                    terminate: false,
                })
            }
            Err(ExtError::ToolFailed { message }) => Err(ToolFailure::msg(message)),
            Err(other) => Err(ToolFailure::msg(format!(
                "extension '{}': {other}",
                self.host.name()
            ))),
        }
    }
}

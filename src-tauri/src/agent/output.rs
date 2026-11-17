//! Bounded assistant output construction — provider/MCP bytes never bypass this layer.

use xfalcon_contracts::credentials::ModelId;
use xfalcon_contracts::RunId;

/// The longest answer one run may produce: exactly what one stored message
/// holds, so a finished answer is never one the conversation cannot keep.
const DEFAULT_OUTPUT_BYTE_BUDGET: usize = crate::chat::domain::MAX_MESSAGE_TEXT_BYTES;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputError {
    ByteBudgetExceeded,
    EmptyOutput,
}

impl std::fmt::Display for OutputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ByteBudgetExceeded => write!(f, "assistant output byte budget exceeded"),
            Self::EmptyOutput => write!(f, "assistant output is empty"),
        }
    }
}

impl std::error::Error for OutputError {}

/// Accumulates streamed assistant text with a hard byte ceiling.
#[derive(Debug)]
pub struct RunOutputBuilder {
    run_id: RunId,
    model_id: ModelId,
    byte_budget: usize,
    bytes_used: usize,
    buffer: String,
    tool_requested: bool,
}

impl RunOutputBuilder {
    #[must_use]
    pub fn new(run_id: RunId, model_id: ModelId, byte_budget: usize) -> Self {
        Self {
            run_id,
            model_id,
            byte_budget: byte_budget.max(1),
            bytes_used: 0,
            buffer: String::new(),
            tool_requested: false,
        }
    }

    #[must_use]
    pub fn with_default_budget(run_id: RunId, model_id: ModelId) -> Self {
        Self::new(run_id, model_id, DEFAULT_OUTPUT_BYTE_BUDGET)
    }

    pub fn note_tool_request(&mut self) -> &mut Self {
        self.tool_requested = true;
        self
    }

    pub fn push_text(&mut self, chunk: &str) -> Result<(), OutputError> {
        let next = self.bytes_used.saturating_add(chunk.len());
        if next > self.byte_budget {
            return Err(OutputError::ByteBudgetExceeded);
        }
        self.buffer.push_str(chunk);
        self.bytes_used = next;
        Ok(())
    }

    #[must_use]
    pub fn had_tool_request(&self) -> bool {
        self.tool_requested
    }

    #[must_use]
    pub fn text_snapshot(&self) -> &str {
        &self.buffer
    }

    pub fn finish(self) -> Result<BuiltRunOutput, OutputError> {
        if self.buffer.is_empty() {
            return Err(OutputError::EmptyOutput);
        }
        Ok(BuiltRunOutput {
            run_id: self.run_id,
            model_id: self.model_id,
            text: self.buffer,
            had_tool_request: self.tool_requested,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuiltRunOutput {
    run_id: RunId,
    model_id: ModelId,
    text: String,
    had_tool_request: bool,
}

impl BuiltRunOutput {
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn had_tool_request(&self) -> bool {
        self.had_tool_request
    }

    #[must_use]
    pub fn run_id(&self) -> &RunId {
        &self.run_id
    }

    #[must_use]
    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }
}

/// Validated final assistant payload for persistence and UI projection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FinalAssistantOutput {
    content: String,
    artifact_refs: Vec<String>,
}

impl FinalAssistantOutput {
    pub fn from_builder(built: BuiltRunOutput) -> Result<Self, OutputError> {
        if built.text().is_empty() {
            return Err(OutputError::EmptyOutput);
        }
        Ok(Self {
            content: built.text().to_owned(),
            artifact_refs: Vec::new(),
        })
    }

    #[must_use]
    pub fn content(&self) -> &str {
        &self.content
    }

    #[must_use]
    pub fn artifact_refs(&self) -> &[String] {
        &self.artifact_refs
    }
}

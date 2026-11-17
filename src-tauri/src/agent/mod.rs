//! Live agent run state machine (distinct from chat-domain [`crate::chat::domain::RunRecord`]).

pub(crate) mod activity;
pub mod context;
pub mod context_budget;
pub mod da_events;
pub mod engine;
pub mod executor;
pub mod journal_recovery;
pub mod local_executor;
pub mod output;
pub mod prompt;
pub mod recovery;
pub mod run;
pub mod stream;
pub(crate) mod summary;
pub mod tool_loop;
pub mod tool_session;
pub mod turn_driver;

pub use context::{ContextBuilder, ContextError, ContextMessage, ContextRole};
pub use engine::{
    run_with_tool_loop, RunEngine, RunEngineError, RunEngineOutcome, ToolLoopRunOutcome,
};
pub use executor::{
    DaPollingExecutor, DaRunObservation, ExecutorError, InferenceExecutor, ModelTurn, ObservedRun,
    ObservedRunEvent, RequestedToolCall, TurnError,
};
pub use journal_recovery::replay_policy_for_journal;
pub use output::{BuiltRunOutput, FinalAssistantOutput, OutputError, RunOutputBuilder};
pub(crate) use prompt::{build_system_prompt, PromptProject, PromptSkill, SystemPromptInputs};
pub use recovery::recover_interrupted_run_policy;
pub use run::{
    InMemoryRunJournal, JournalEntry, Run, RunBudgets, RunError, RunJournal, TransitionIntent,
    TransitionOutcome,
};
pub use stream::{CoalesceConfig, InferencePresentationSink, StreamError};
pub use tool_loop::{
    effective_registry_for_run, last_journaled_status, ModelToolCall, RoundOutcome,
    ToolCallOutcome, ToolExecutionError, ToolExecutorPort, ToolLoop, ToolLoopAudit,
    ToolLoopBudgets, ToolLoopError, MAX_CALLS_PER_TOOL, MAX_PARALLEL_TOOL_CALLS,
    MAX_TOOL_CALLS_PER_RUN, MAX_TOOL_ROUNDS, ROUND_TIMEOUT, TOOL_CALL_TIMEOUT,
};
pub(crate) use tool_session::{build_tool_session, ToolSessionRequest};
pub(crate) use turn_driver::{LocalTurnDriver, LocalTurnDriverConfig};

#[cfg(test)]
mod tests;

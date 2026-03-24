pub mod claude;
pub mod codex;
pub mod gemini;
pub mod mock;
pub mod ollama;

use async_trait::async_trait;

use crate::{
    error::Result,
    spec::AgentSpec,
    types::{CompiledContext, RunRequest},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerTerminalState {
    Succeeded,
    Failed { message: String },
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone)]
pub struct RunnerExecution {
    pub terminal_state: RunnerTerminalState,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution>;
}

#[async_trait]
impl<T> AgentRunner for Box<T>
where
    T: AgentRunner + ?Sized,
{
    async fn execute(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        (**self).execute(spec, request, compiled).await
    }
}

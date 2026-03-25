pub mod claude;
pub mod codex;
pub mod gemini;
pub mod mock;
pub mod ollama;
pub mod streaming;

use async_trait::async_trait;

use crate::{
    error::Result,
    spec::AgentSpec,
    types::{CompiledContext, TaskSpec, WorkflowHints},
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunnerTerminalState {
    Succeeded,
    Failed { message: String },
    TimedOut,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunnerOutputStream {
    Stdout,
    Stderr,
}

pub trait RunnerOutputObserver: Send {
    fn on_output(&mut self, stream: RunnerOutputStream, chunk: &str);
}

#[derive(Debug, Clone)]
pub struct RunnerExecution {
    pub terminal_state: RunnerTerminalState,
    pub stdout: String,
    pub stderr: String,
}

#[async_trait]
pub trait AgentRunner: Send + Sync {
    async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution>;

    async fn execute_task_with_observer(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
        _observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        self.execute_task(spec, task_spec, hints, compiled).await
    }
}

#[async_trait]
impl<T> AgentRunner for Box<T>
where
    T: AgentRunner + ?Sized,
{
    async fn execute_task(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
    ) -> Result<RunnerExecution> {
        (**self)
            .execute_task(spec, task_spec, hints, compiled)
            .await
    }

    async fn execute_task_with_observer(
        &self,
        spec: &AgentSpec,
        task_spec: &TaskSpec,
        hints: &WorkflowHints,
        compiled: &CompiledContext,
        observer: &mut dyn RunnerOutputObserver,
    ) -> Result<RunnerExecution> {
        (**self)
            .execute_task_with_observer(spec, task_spec, hints, compiled, observer)
            .await
    }
}

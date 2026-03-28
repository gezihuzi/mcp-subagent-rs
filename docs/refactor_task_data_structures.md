# 子代理任务数据结构重构设计 v0.11

## 目标

将当前 ~50 个 struct/enum 按生命周期分成三个正交层级：

```
TaskSpec (前置不可变)  →  RunState (运行中可变缓存)  →  RunOutcome (终态不可变)
```

不做旧版本兼容。

---

## 现状问题

以 run `019d25a6` 为例：

1. `record.summary.as_ref().map(|s| s.summary.summary.clone())` — 3 层穿透取一句话摘要
2. `RunRecord` 用 16 个 `Option` 表达阶段，读者无法判断"当前到了哪步"
3. `tools.rs:run_agent` 手动拼 16 字段（L1335-1429）；`spawn_agent` 430 行逐字段 mutation
4. `DispatchResult` 携带 raw stdout/stderr，消费方只需结构化结果
5. DTO 层 `RunAgentOutput`/`AgentStatusOutput`/`GetRunResultOutput` 三种不一致的 output
6. 每条 run 落盘 18 个文件，大量是 `run.json` 的冗余子集

---

## 新设计

### 1. TaskSpec — 前置不可变（替代 RunRequest）

```rust
pub struct TaskSpec {
    pub task: String,
    pub task_brief: Option<String>,
    pub acceptance_criteria: Vec<String>,
    pub selected_files: Vec<SelectedFile>,
    pub working_dir: PathBuf,
}

pub struct WorkflowHints {
    pub stage: Option<String>,
    pub plan_ref: Option<String>,
    pub parent_summary: Option<String>,
    pub run_mode: RunMode,
}
```

### 2. RunState — 运行中可变（替代 RunRecord 的 16 个 Option）

```rust
pub enum RunPhase {
    Queued,
    Validating,
    ProbingProvider,
    PreparingWorkspace,
    ResolvingMemory,
    CompilingContext,
    Launching,
    Running { attempt: u32 },
    Collecting,
    ParsingSummary,
    Finalizing,
}

pub struct RunState {
    pub handle_id: Uuid,
    pub agent_name: String,
    pub task_brief: Option<String>,
    pub phase: RunPhase,
    pub phase_history: Vec<PhaseEntry>,
    pub created_at: OffsetDateTime,
    pub updated_at: OffsetDateTime,
    pub probe: Option<ProbeSnapshot>,
    pub workspace: Option<WorkspaceSnapshot>,
    pub memory: Option<MemorySnapshot>,
    pub context_digest: Option<String>,
    pub policy: Option<PolicySnapshot>,
}
```

### 3. RunOutcome — 终态不可变（替代 DispatchResult+SummaryEnvelope 组合）

```rust
pub enum RunOutcome {
    Succeeded(SuccessOutcome),
    Failed(FailureOutcome),
    Cancelled { reason: String },
    TimedOut { elapsed_secs: u64 },
}

pub struct SuccessOutcome {
    pub summary: String,
    pub key_findings: Vec<String>,
    pub touched_files: Vec<String>,
    pub next_steps: Vec<String>,
    pub open_questions: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
    pub verification: VerificationStatus,
    pub usage: UsageStats,
    pub parse_status: SummaryParseStatus,
}

pub struct FailureOutcome {
    pub error: String,
    pub retry: RetryInfo,
    pub partial_summary: Option<String>,
    pub usage: UsageStats,
}

pub struct RetryInfo {
    pub classification: RetryClassification,
    pub reason: Option<String>,
    pub attempts_used: u32,
}

pub struct UsageStats {
    pub duration_ms: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub provider_exit_code: Option<i32>,
}
```

### 4. 统一 MCP DTO（替代 4 种 output struct）

```rust
pub struct RunView {
    pub handle_id: String,
    pub agent_name: String,
    pub task_brief: Option<String>,
    pub phase: String,
    pub terminal: bool,
    pub outcome: Option<OutcomeView>,
    pub created_at: String,
    pub updated_at: String,
}

pub enum OutcomeView {
    Succeeded { summary, key_findings, touched_files, artifacts, usage },
    Failed { error, retry_classification, partial_summary, usage },
    Cancelled { reason },
    TimedOut { elapsed_secs },
}
```

### 5. 持久化简化

落盘文件从 18 个减到 5 个：

| 文件 | 内容 |
|---|---|
| `run.json` | `PersistedRun { task_spec, hints, state, outcome, spec_snapshot }` |
| `events.jsonl` | 事件流 |
| `stdout.log` | provider stdout |
| `stderr.log` | provider stderr |
| `compiled-context.md` | 编译后 prompt（调试用） |

---

## Struct 归属判定

### 保留不变
`Provider`, `AgentSpecCore`, `AgentSpec`, `RuntimePolicy` + 16 enums, `WorkflowSpec` + 5 sub-policies, `ProviderOverrides`, `CompiledContext`, `ResolvedMemory`, `MemorySnippet`, `ContextSourceRef`, `InjectionMode`, `ArtifactRef`, `RunnerExecution`, `RunnerTerminalState`

### 重构
- `RunRequest` → `TaskSpec` + `WorkflowHints`
- `RunMetadata` + `RunStatus` → `RunState` + `RunPhase`
- `RunRecord` (16 Options) → `RunState` + typed caches
- `DispatchResult` + `SummaryEnvelope` + `StructuredSummary` → `RunOutcome`
- `RunAgentOutput`/`AgentStatusOutput`/`GetRunResultOutput`/`SpawnAgentOutput`/`SummaryOutput` → `RunView` + `OutcomeView`

### 删除
`RunRequestSnapshot`, `PersistedRunRecord`（合并到 `PersistedRun`）, `RetryClassificationRecord`（进 `RetryInfo`）, `ExecutionPolicyRecord`（进 `PolicySnapshot`）, `DispatchEnvelope`（简化）

---

## 迁移步骤

1. 新建 `src/runtime/outcome.rs`
2. 重写 `src/types.rs`
3. 重写 `src/mcp/state.rs`
4. 大改 `src/runtime/dispatcher.rs`
5. 调整 `src/runtime/summary.rs`
6. 签名链：`context.rs` → `memory.rs` → `runners/mod.rs` → 5 runners
7. 重写 `src/mcp/dto.rs`
8. 大改 `src/mcp/tools.rs`
9. 大改 `src/mcp/service.rs` + `persistence.rs`
10. 适配 `src/main.rs` + tests

## 受影响文件

17 个文件，估计 5000-7000 行变更（含测试），核心逻辑 ~3000 行。

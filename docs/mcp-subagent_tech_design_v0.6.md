# mcp-subagent / mcp-subagent-rs

## 技术设计文档 v0.6（实现收口版 / 可直接落地）

- **crate 名称**：`mcp-subagent`
- **仓库名**：`mcp-subagent-rs`
- **二进制名称**：`mcp-subagent`
- **文档状态**：**实现驱动规格**（implementation-driving spec）
- **适用阶段**：在现有仓库基础上继续开发、收口、上线本地可运行版本
- **目标读者**：维护者、Rust 开发者、MCP 接入方、子代理工作流设计者

---

## 0. 本版拍板结论

这版不是继续“讨论方向”，而是直接拍板。以下内容视为本项目后续实现的正式决策：

1. **保留现有主架构**：单一 Rust 二进制，外层 MCP Server，内层 Local Agent Runtime。
2. **`context_mode` 必须变成真实行为**，不能只停留在 schema 和 prompt 文案里。
3. **引入统一工作流层**，将 `Research -> Plan -> Build -> Review -> Archive` 作为 runtime 一等能力，而不是团队口头习惯。
4. **`PLAN.md` 升级为公共协作工件**，并引入 `ActivePlan` 作为高优先级 memory source。
5. **默认多 Agent 写代码任务优先 `git_worktree`**，失败再退化到 `temp_copy`；只读任务可以直接 `in_place`。
6. **`src/mcp/server.rs` 必须拆模块**，避免继续膨胀成“万能文件”。
7. **真实 provider runner 必须统一进同一个 trait**，不能长期维持 mock 走统一抽象、真实 provider 走分叉逻辑。
8. **支持分 Tier 管理**：Codex 为主实现路径，Claude 次之，Gemini 明确实验性，Ollama 预留但不假装“已完成”。
9. **本版本要求“本地可跑”**：除 MCP 模式外，必须提供本地调试 CLI 子命令，便于不接 Host 直接验证。
10. **禁止 raw transcript 透传** 是 MUST，不允许任何“图方便”绕过 ContextCompiler。

---

## 1. 背景与设计依据

本项目的根本目标不是“把多个厂商 API 包一层”，而是：

> 在一台本机设备上，用统一的 Rust runtime 调度已安装的 provider CLI（如 `codex` / `claude` / `gemini`），
> 让多个 LLM 以可控的上下文边界、统一的产物格式、可观测的执行状态进行协作。

### 1.1 关键现实约束

- **通过本地 CLI 调用，不等于 provider 天然离线**。`claude`、`codex`、`gemini` 都是本地客户端，但是否联网、如何认证、模型实际运行位置，仍取决于底层 provider/CLI。
- **三家当前公开的 subagent 方向一致**：强调独立上下文窗口、减少主线程污染、将摘要回传给主流程，而不是把主线程完整历史灌给子代理。
- **本项目不是 provider-native subagent 的包装器**。它是一个更高一层的、本地可控的 Agent Runtime + Workflow Runtime。
- **真正难点不只是“怎么 spawn CLI”**，而是“多 Agent 围绕什么协作、如何共享稳定事实、如何避免上下文漂移和文件冲突”。

### 1.2 本版要解决的核心问题

1. 现有仓库已有 runtime 骨架，但 **schema 已领先行为**，尤其是 `context_mode`、`spawn_policy`、`approval`、`background` 等字段。
2. 多 Agent 调度内核已有雏形，但 **缺少统一工作流层**，导致协作锚点仍偏 prompt 驱动。
3. MCP surface 已成型，但 **server 模块复杂度上升过快**。
4. 多 provider runner 已接入，但 **成熟度不同，需要明确支持层级和降级策略**。

---

## 2. 项目目标与非目标

### 2.1 目标

`mcp-subagent` 是一个本地优先（local-first）的 Agent Workflow Runtime，具备以下能力：

- 以 **Rust 单一二进制** 运行。
- 对外暴露为 **标准 MCP Server**，兼容 Claude Desktop / Cursor / 任意 MCP Host。
- 对内调度已安装的 provider CLI：`codex` / `claude` / `gemini` / 未来 `ollama`。
- 使用统一 schema 描述 Agent、工作流、上下文策略、工作目录策略、冲突策略和 summary contract。
- 允许多个 Agent 在 **共享项目事实、隔离执行上下文、独立工作目录** 的前提下协作。
- 将输出统一收敛为结构化 summary、产物索引、运行状态、日志和归档知识。

### 2.2 非目标

本版本明确不做以下事情：

- 不把 raw transcript 作为共享记忆层。
- 不以“长期维护 vendor 原生 agent 配置文件”为主路径。
- 不做分布式调度、集群执行或远程工作节点。
- 不做 GUI。
- 不把 Ollama 写成“已一等支持”直到真实 runner、probe、doctor、e2e 都完成。
- 不承诺完全对齐每家 provider 的所有交互能力；以稳定、本地可验证的 CLI 路径为准。

---

## 3. 支持层级（Support Tiers）

本项目后续开发按层级推进，不再把所有 provider 都写成同成熟度：

| Tier | Provider | 状态 | 说明 |
|---|---|---:|---|
| T0 | Mock | Stable | 测试、CI、本地无 provider 时的保底路径 |
| T1 | Codex | Primary | 作为首个重点打磨的真实 runner；非交互 CLI、输出 schema、sandbox/approval 映射最清晰 |
| T1.5 | Claude | Beta | 真实 runner 支持，但需谨慎校准权限/输出/工作目录行为 |
| T2 | Gemini | Experimental | 明确实验性；支持可用但不承诺与 Codex/Claude 同级稳定性 |
| T2 | Ollama | Reserved | 预留 provider 枚举和 capability 占位；没有真实 runner 之前不得宣传为“已支持” |

### 3.1 版本承诺

- **MVP 可运行版本必须满足**：Mock + Codex 真正稳定可跑；Claude 可跑但允许 Beta；Gemini 标记 experimental；Ollama 仅预留。
- **上线默认路径**：先让 `doctor`、`validate`、`run`、`--mcp` 在 Mock/Codex 路径上稳定，再逐步补齐其他 provider。

---

## 4. 核心产品定义

### 4.1 产品定位

`mcp-subagent` 不是“多模型聊天壳”，而是：

> **Local Agent Workflow Runtime**
>
> - 运行层：spawn 本地 CLI，管理 workspace、状态、日志、artifact、summary
> - 协作层：plan 驱动、多阶段执行、阶段感知分发、审查与归档
> - 集成层：通过 MCP 对外暴露统一工具面

### 4.2 设计原则

1. **默认隔离**：默认不共享完整历史。
2. **显式注入**：只有编译后的 task brief / parent summary / selected files / active plan / memory sources 可以注入。
3. **结构化回传**：所有执行都要落回统一 summary contract。
4. **工件驱动**：共享的是 `PLAN.md`、summary、artifacts、docs/decisions，而不是聊天记录。
5. **本地优先**：调试和开发应当在本地直接运行，不依赖外部 Host 才能验证。
6. **能力分层**：provider 能力不一致时，runtime 负责统一抽象和降级，而不是假装一切都支持。

---

## 5. 总体架构

```text
mcp-subagent
├── CLI / entrypoint
│   ├── mcp-subagent --mcp
│   ├── mcp-subagent doctor
│   ├── mcp-subagent validate
│   ├── mcp-subagent list-agents
│   ├── mcp-subagent run
│   ├── mcp-subagent spawn
│   ├── mcp-subagent status
│   ├── mcp-subagent cancel
│   └── mcp-subagent artifact
│
├── MCP Server Layer (rmcp)
│   ├── tools surface
│   ├── DTO / schema mapping
│   ├── run state facade
│   └── persistence facade
│
└── Runtime Layer
    ├── Spec / validation
    ├── Probe / doctor
    ├── Workflow engine
    ├── Context compiler
    ├── Memory resolver
    ├── Workspace manager
    ├── Dispatcher
    ├── Runner registry
    ├── Provider runners
    ├── Summary parser / validator
    └── State / artifacts / logs
```

### 5.1 与当前仓库的关系

现有仓库已经有：spec 分层、runtime、probe、doctor、workspace、summary、dispatcher、MCP surface 等核心骨架。

本版文档不是推翻重来，而是把它收紧成：

- 更明确的行为约束
- 更清晰的模块边界
- 更可控的工作流层
- 更可实施的本地运行路径

---

## 6. 建议代码结构（拍板版）

```text
src/
├── main.rs
├── cli/
│   ├── mod.rs
│   ├── commands.rs
│   └── output.rs
├── mcp/
│   ├── mod.rs
│   ├── service.rs
│   ├── tools.rs
│   ├── dto.rs
│   ├── state.rs
│   ├── persistence.rs
│   └── artifacts.rs
├── spec/
│   ├── mod.rs
│   ├── core.rs
│   ├── runtime_policy.rs
│   ├── provider_overrides.rs
│   ├── workflow.rs
│   ├── registry.rs
│   └── validate.rs
├── probe/
│   ├── mod.rs
│   ├── provider.rs
│   └── capability.rs
├── doctor/
│   ├── mod.rs
│   └── report.rs
├── runtime/
│   ├── mod.rs
│   ├── dispatcher.rs
│   ├── runner.rs
│   ├── context.rs
│   ├── memory.rs
│   ├── summary.rs
│   ├── workspace.rs
│   ├── execution.rs
│   ├── status.rs
│   ├── artifacts.rs
│   ├── locks.rs
│   ├── cleanup.rs
│   ├── workflow/
│   │   ├── mod.rs
│   │   ├── gate.rs
│   │   ├── active_plan.rs
│   │   ├── stage.rs
│   │   ├── template.rs
│   │   ├── archive.rs
│   │   └── knowledge_capture.rs
│   └── runners/
│       ├── mod.rs
│       ├── mock.rs
│       ├── codex.rs
│       ├── claude.rs
│       ├── gemini.rs
│       └── ollama.rs   # 可保留占位，但功能未完成前标记 reserved
└── util/
    ├── paths.rs
    ├── fs.rs
    ├── json.rs
    └── time.rs
```

### 6.1 必拆文件

`src/mcp/server.rs` 必须拆分，最低要求拆为：

- `mcp/service.rs`：rmcp server 装配与生命周期
- `mcp/tools.rs`：tool 入口函数
- `mcp/state.rs`：运行句柄、状态查询、取消
- `mcp/persistence.rs`：状态与运行记录落盘
- `mcp/dto.rs`：输入输出结构
- `mcp/artifacts.rs`：artifact 读取与索引

> **拍板理由**：当前 MCP 层不应同时承担 transport、tool schema、状态机、持久化、artifact 读取等所有职责。

---

## 7. 统一 Spec 模型

### 7.1 顶层结构

```rust
pub struct AgentSpec {
    pub core: AgentSpecCore,
    pub runtime: RuntimePolicy,
    pub provider_overrides: ProviderOverrides,
    pub workflow: Option<WorkflowSpec>,
}
```

### 7.2 Core：稳定跨 provider 字段

```rust
pub struct AgentSpecCore {
    pub name: String,
    pub description: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub role: AgentRole,
    pub instructions: String,
    pub tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub skills: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub metadata: std::collections::HashMap<String, serde_json::Value>,
}
```

### 7.3 RuntimePolicy：执行策略

```rust
pub struct RuntimePolicy {
    pub context_mode: ContextMode,
    pub memory_sources: Vec<MemorySource>,
    pub sandbox: SandboxMode,
    pub approval: ApprovalMode,
    pub working_dir_policy: WorkingDirPolicy,
    pub file_conflict_policy: FileConflictPolicy,
    pub timeout_secs: u64,
    pub max_turns: Option<u32>,
    pub background_preference: BackgroundPreference,
    pub spawn_policy: SpawnPolicy,
    pub isolation: IsolationMode,
}
```

### 7.4 ProviderOverrides：厂商特有字段

```rust
pub struct ProviderOverrides {
    pub claude: Option<ClaudeOverrides>,
    pub codex: Option<CodexOverrides>,
    pub gemini: Option<GeminiOverrides>,
    pub ollama: Option<OllamaOverrides>,
}
```

### 7.5 WorkflowSpec：新增一等工作流层

```rust
pub struct WorkflowSpec {
    pub enabled: bool,
    pub require_plan_when: WorkflowGatePolicy,
    pub stages: Vec<WorkflowStageKind>,
    pub active_plan: ActivePlanPolicy,
    pub review_policy: ReviewPolicy,
    pub knowledge_capture: KnowledgeCapturePolicy,
    pub archive_policy: ArchivePolicy,
    pub max_runtime_depth: u8,
    pub allowed_stages: Vec<WorkflowStageKind>,
}
```

### 7.6 新增核心枚举（拍板版）

```rust
pub enum Provider {
    Mock,
    Codex,
    Claude,
    Gemini,
    Ollama,
}

pub enum AgentRole {
    Planner,
    Explorer,
    Coder,
    ReviewerCorrectness,
    ReviewerStyle,
    Summarizer,
    GeneralPurpose,
}

pub enum ContextMode {
    Isolated,
    SummaryOnly,
    SelectedFiles(Vec<String>),
    ExpandedBrief,
}

pub enum MemorySource {
    AutoProjectMemory,
    ActivePlan,
    File(std::path::PathBuf),
    Glob(String),
    ArchivedPlans,
}

pub enum WorkingDirPolicy {
    Auto,
    InPlace,
    TempCopy,
    GitWorktree,
}

pub enum FileConflictPolicy {
    Deny,
    Serialize,
    AllowWithMergeReview,
}

pub enum SandboxMode {
    ReadOnly,
    WorkspaceWrite,
    DangerFullAccess,
}

pub enum ApprovalMode {
    Default,
    OnRequest,
    Never,
}

pub enum BackgroundPreference {
    ForegroundPreferred,
    BackgroundPreferred,
    ProviderManagedIfAvailable,
}

pub enum SpawnPolicy {
    Forbid,
    AllowDirectChild,
    WorkflowOnly,
}

pub enum IsolationMode {
    SharedRepo,
    IsolatedWorkspace,
}

pub enum WorkflowStageKind {
    Research,
    Plan,
    Build,
    Review,
    Archive,
}
```

### 7.7 默认值（正式拍板）

```rust
impl Default for RuntimePolicy {
    fn default() -> Self {
        Self {
            context_mode: ContextMode::Isolated,
            memory_sources: vec![
                MemorySource::AutoProjectMemory,
                MemorySource::ActivePlan,
            ],
            sandbox: SandboxMode::ReadOnly,
            approval: ApprovalMode::OnRequest,
            working_dir_policy: WorkingDirPolicy::Auto,
            file_conflict_policy: FileConflictPolicy::Serialize,
            timeout_secs: 900,
            max_turns: None,
            background_preference: BackgroundPreference::ForegroundPreferred,
            spawn_policy: SpawnPolicy::WorkflowOnly,
            isolation: IsolationMode::IsolatedWorkspace,
        }
    }
}
```

> **关键改动**：默认 `working_dir_policy` 从固定 `TempCopy` 改为 `Auto`。
>
> 解析逻辑见第 11 章。

---

## 8. 验证规则（Validate 必须硬编码的约束）

### 8.1 MUST 级约束

1. **禁止 raw transcript 透传**。
2. `ContextMode::SelectedFiles` 中的路径必须存在于工作区或显式允许的附加目录中。
3. `SpawnPolicy::Forbid` 时，任何内部再派发都必须失败。
4. `WorkflowSpec.enabled = true` 且进入 `Build` / `Review` 阶段时，如果满足 gate 条件但不存在 `PLAN.md`，运行必须失败或转为 `plan required` 状态。
5. `WorkingDirPolicy::GitWorktree` 在非 git 仓库下必须失败；`Auto` 可回退到 `TempCopy`。
6. `Provider::Ollama` 在未实现真实 runner 前必须标记为 `reserved`，不能通过 `doctor` 报告成 fully supported。
7. `max_runtime_depth` 默认 `1`；即允许直接 child，不允许 child 再递归 child。
8. 如果 provider 能原生加载项目记忆文件（例如 `CLAUDE.md` / `AGENTS.md` / `GEMINI.md`），runtime 默认**不重复 inline**相同文件内容。

### 8.2 SHOULD 级约束

1. 对结构化 summary 使用 JSON Schema 做强校验；无法原生支持时做后置校验。
2. 对 write 任务优先用隔离工作目录。
3. 对大仓库在 `doctor` 中给出 workspace 策略成本提示。
4. 对有冲突风险的并行写入默认串行化。

---

## 9. 上下文与记忆模型（正式版）

这是本项目最核心的一章。

### 9.1 总原则

- Runtime **MUST NOT** forward raw parent transcript to child agents.
- 共享的是稳定事实和压缩工件，不是聊天噪声。
- 子代理只应得到完成任务所需的最小上下文集。
- `PLAN.md` 是共享任务事实层，优先级高于临时 parent summary。
- provider-native memory 文件默认通过 provider 自身发现机制生效，避免双重注入。

### 9.2 允许注入的上下文类型

允许注入到 child agent 的只有以下几类：

1. **Task Brief**：本次任务目标、范围、约束、验收标准
2. **Parent Summary**：父代理压缩后的摘要，不包含原始 transcript
3. **Selected Files**：显式指定的文件内容片段
4. **Project Memory**：`PROJECT.md`、显式 memory files 等
5. **Provider-native Memory**：`CLAUDE.md` / `AGENTS.md` / `GEMINI.md` 等，由 provider 原生发现或 runtime fallback inline
6. **Active Plan**：当前任务的 `PLAN.md` 或其切片摘要
7. **Archived Knowledge**：归档后的 plan / decisions / final summaries（低优先级）

### 9.3 `context_mode` 必须对应真实注入行为

> 这是当前仓库 P0 修复项之一。

#### `Isolated`

**允许：**

- task brief
- project memory
- active plan 相关 section（若 workflow 开启）

**禁止：**

- parent summary
- selected files 正文
- raw transcript

#### `SummaryOnly`

**允许：**

- task brief
- project memory
- active plan 相关 section
- parent summary

**禁止：**

- selected files 正文
- raw transcript

#### `SelectedFiles(Vec<String>)`

**允许：**

- task brief
- project memory
- active plan 相关 section
- 指定文件的正文片段

**禁止：**

- parent summary（默认不带，避免又大又杂）
- raw transcript

#### `ExpandedBrief`

**允许：**

- task brief（扩展版）
- project memory
- active plan 相关 section
- 编译器生成的 extended background
- compiler 生成的 parent summary digest

**禁止：**

- raw transcript

### 9.4 ContextCompiler 的正式职责

```rust
#[async_trait::async_trait]
pub trait ContextCompiler: Send + Sync {
    async fn compile(
        &self,
        req: &CompileRequest,
    ) -> anyhow::Result<CompiledContext>;

    async fn parse_summary(
        &self,
        raw_output: &str,
        schema_version: &str,
    ) -> anyhow::Result<SummaryEnvelope>;
}
```

```rust
pub struct CompileRequest {
    pub agent_spec: AgentSpec,
    pub task: String,
    pub task_brief: Option<String>,
    pub parent_summary: Option<String>,
    pub selected_files: Vec<std::path::PathBuf>,
    pub active_plan: Option<ActivePlanRef>,
    pub workspace: WorkspaceContext,
}

pub struct CompiledContext {
    pub system_prefix: String,
    pub task_prompt: String,
    pub injected_files: Vec<InjectedFile>,
    pub active_plan_excerpt: Option<String>,
    pub memory_manifest: Vec<ResolvedMemoryItem>,
    pub notes: Vec<String>,
}
```

### 9.5 编译流程（固定四步升级为六步）

1. 解析 `context_mode`
2. 解析 memory sources（含 `ActivePlan`）
3. 去重 + provider-native memory passthrough 决策
4. 生成 task brief / extended brief / plan excerpt
5. 生成 provider-neutral compiled context
6. 交给 runner 做 provider-specific 映射

### 9.6 provider-native memory 的处理规则（正式拍板）

对于支持自动加载项目记忆文件的 provider，runtime 默认不重复 inline：

- Claude：`CLAUDE.md` 相关文件由 CLI 自身发现时，优先依赖原生发现
- Codex：`AGENTS.md` 由 CLI 原生读取时，不再重复 inline
- Gemini：`GEMINI.md` 原生可读时，不再重复 inline

#### fallback inline 的条件

只有满足以下任一条件时，runtime 才可以 inline 原生记忆文件：

- 该文件不在 provider 实际工作目录内
- 当前 workspace 是临时副本，provider 无法发现原路径内容
- provider probe 表明当前版本/模式未启用原生记忆
- 调试模式显式要求 `force_inline_native_memory = true`

### 9.7 `ActivePlan` 作为一等 memory source

这是本版新增的关键设计。

#### 设计结论

- 当 workflow 启用且当前任务存在 `PLAN.md` 时，`ActivePlan` 默认进入 memory sources。
- 子代理 summary 必须引用其执行对应的 `plan step` 或 `section`。
- `PLAN.md` 完成后被归档，并从 active memory 降级为 archived knowledge source。

#### 作用

- 提供稳定共享事实层
- 减少 parent summary 漂移
- 提升跨 session / 跨模型 / 跨 provider 连续性
- 让多 Agent 协作从“聊天驱动”转为“工件驱动”

---

## 10. 统一工作流层（Workflow Runtime）

这一层是本版最大的新增内容，也是“能长期用”和“只能跑”之间的分水岭。

### 10.1 统一工作流不是建议，是内建能力

本项目不再把以下规则当作团队口头习惯，而是纳入 runtime：

- 复杂任务必须先有 plan
- 多 Agent 围绕 plan 的 section 协作
- build 之后必须 review
- 完成后必须 archive / knowledge capture

### 10.2 标准阶段

```rust
pub enum WorkflowStageKind {
    Research,
    Plan,
    Build,
    Review,
    Archive,
}
```

### 10.3 每个阶段的职责

#### Research

- 搜集事实
- 查找代码路径
- 识别风险与备选方案
- **不得修改文件**

#### Plan

- 生成/更新 `PLAN.md`
- 明确范围、非目标、验收标准、回滚策略
- 绑定后续 build/review 的工件锚点

#### Build

- 按 plan 执行
- 只允许修改与 plan 对齐的区域
- summary 必须回填 `plan_refs`

#### Review

- 依据 `acceptance criteria`、`review checklist` 和 diff 进行审查
- 至少支持 correctness review
- style review 可配置启用

#### Archive

- 归档完成后的 plan / final summary / decision notes
- 触发知识沉淀

### 10.4 WorkflowGatePolicy（复杂度门槛）

不能只用“>5 文件”这一个条件。正式设计如下：

```rust
pub struct WorkflowGatePolicy {
    pub require_plan_if_touched_files_ge: Option<u32>,
    pub require_plan_if_cross_module: bool,
    pub require_plan_if_parallel_agents: bool,
    pub require_plan_if_new_interface: bool,
    pub require_plan_if_migration: bool,
    pub require_plan_if_human_approval_point: bool,
    pub require_plan_if_estimated_runtime_minutes_ge: Option<u32>,
}
```

### 10.5 默认 gate 策略（拍板）

满足任一条件则必须先有 `PLAN.md`：

- 预期修改 `>= 5` 个文件
- 涉及跨模块/跨 crate
- 需要并行子代理
- 需要新增接口/配置/迁移
- 需要人工审批点
- 预计执行时间 `>= 15` 分钟

### 10.6 `PLAN.md` 固定模板

`PLAN.md` 不再是随意文本，而是协议工件。

```markdown
---
id: plan-YYYYMMDD-<slug>
status: draft | active | blocked | completed | archived
repo: mcp-subagent-rs
owners: []
agents: []
created_at: 2026-03-24
updated_at: 2026-03-24
---

# Goal

# Scope

# Non-goals

# Constraints

# Affected areas

# Research findings

# Execution steps
- [ ] Step 1
- [ ] Step 2

# Acceptance criteria

# Rollback / fallback

# Review checklist

# Artifacts

# Final summary
```

### 10.7 `PLAN.md` 的一等地位

- planner 负责生成初稿
- human 或主代理负责批准
- coder 必须引用 plan step 执行
- reviewer 必须引用 acceptance criteria 审查
- archiver / summarizer 负责归档

### 10.8 Stage-aware Dispatcher

Dispatcher 不能再只按 provider / name 路由，还要按阶段路由。

```rust
pub struct DispatchRequest {
    pub stage: WorkflowStageKind,
    pub agent_name: String,
    pub task: String,
    pub active_plan: Option<ActivePlanRef>,
    pub summary_of_previous_stage: Option<String>,
}
```

#### 阶段与角色默认映射

| Stage | 优先角色 |
|---|---|
| Research | Planner / Explorer |
| Plan | Planner |
| Build | Coder / GeneralPurpose |
| Review | ReviewerCorrectness / ReviewerStyle |
| Archive | Summarizer |

### 10.9 ReviewPolicy（不绑定具体厂商）

不写死“Codex + Sonnet”这类模型组合。正式抽象如下：

```rust
pub struct ReviewPolicy {
    pub require_correctness_review: bool,
    pub require_style_review: bool,
    pub allow_same_provider_dual_review: bool,
    pub prefer_cross_provider_review: bool,
}
```

#### 默认值

- correctness review：开启
- style review：可选
- 优先跨 provider，但不强制

### 10.10 KnowledgeCapturePolicy

把“值得记录吗？”从人工习惯改成 runtime hook。

```rust
pub struct KnowledgeCapturePolicy {
    pub trigger_if_touched_files_gt: Option<u32>,
    pub trigger_if_new_config: bool,
    pub trigger_if_behavior_change: bool,
    pub trigger_if_non_obvious_bugfix: bool,
    pub write_decision_note: bool,
    pub update_project_memory: bool,
}
```

#### 默认触发条件

满足任一条件时，运行结束后提示或自动生成 capture：

- `touched_files > 3`
- 新增配置 / 命令 / 行为变化
- 修复非显然 bug
- 产出新的 workflow 经验或仓库约定

### 10.11 ArchivePolicy

```rust
pub struct ArchivePolicy {
    pub enabled: bool,
    pub archive_dir: std::path::PathBuf,
    pub write_final_summary: bool,
    pub write_metadata_index: bool,
}
```

归档目标：

- `docs/plans/<date>-<slug>.md`
- `docs/plans/index.json`
- `docs/decisions/<date>-<slug>.md`（必要时）

---

## 11. Workspace 策略（正式改版）

这部分直接决定多 Agent 写代码时是否会“慢死或打架”。

### 11.1 `WorkingDirPolicy::Auto` 的正式解析规则

默认不再固定 `TempCopy`，而是改为 `Auto`：

#### 只读任务

- `sandbox = ReadOnly`
- `stage in {Research, Plan}`
- 无文件修改预期

**默认：** `InPlace`

#### 写任务（git 仓库 + worktree 可用）

- `sandbox != ReadOnly`
- `stage in {Build, Review}`
- 仓库是 git
- `git worktree` 可用

**默认：** `GitWorktree`

#### 写任务（无法 worktree）

- 非 git 仓库
- git worktree 不可用
- 临时降级

**默认：** `TempCopy`

### 11.2 为什么这样拍板

- 只读任务无需复制整个仓库
- 并行写任务使用 worktree 更适合真实仓库协作
- `TempCopy` 作为安全兜底仍保留
- 这比“永远 temp_copy”更能承受大仓库和并发

### 11.3 FileConflictPolicy

#### `Deny`

如果已有其它运行持有相同 repo 写锁，则直接拒绝。

#### `Serialize`

同仓库写任务串行执行。

#### `AllowWithMergeReview`

允许并行隔离工作区写入，但必须产出 merge-review 工件；主代理或人工决定合并。

### 11.4 MVP 默认冲突策略

默认 `Serialize`。

#### 说明

- 这不是最激进的并行方案，但最可预测。
- 等 `touched_files` 预测和更细粒度锁成熟后，再考虑 repo 内不同目录的并行。

### 11.5 Cleanup 必须形成闭环

运行结束后必须自动清理：

- temp workspace
- git worktree
- 临时 provider config 文件
- 临时 summary schema 文件

保留内容：

- logs
- summary
- artifacts
- status snapshot
- 需要审计的 workspace metadata

---

## 12. 统一 Summary Contract（强约束版）

### 12.1 目标

所有 provider 最终都要收敛为相同 summary envelope，不能“每家差不多”。

### 12.2 正式结构

```rust
pub struct SummaryEnvelope {
    pub contract_version: String,
    pub parse_status: SummaryParseStatus,
    pub summary: StructuredSummary,
    pub raw_fallback_text: Option<String>,
}

pub struct StructuredSummary {
    pub summary: String,
    pub key_findings: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
    pub exit_code: i32,
    pub verification_status: VerificationStatus,
    pub touched_files: Vec<String>,
    pub plan_refs: Vec<String>,
}

pub struct ArtifactRef {
    pub kind: String,
    pub path: std::path::PathBuf,
    pub description: Option<String>,
}

pub enum SummaryParseStatus {
    Validated,
    Degraded,
    Invalid,
}

pub enum VerificationStatus {
    NotRun,
    Passed,
    Failed,
    Partial,
}
```

### 12.3 结构化输出策略

#### Codex

优先使用 `--output-schema`；必要时同时写 `--output-last-message` 以便保留最终文本。

#### Claude

优先使用 `--json-schema`（print mode）；保留原始文本作为 fallback。

#### Gemini

当前按 prompt contract + 后置 JSON 解析校验处理；不把未验证结构假装成 validated。

### 12.4 Parse Failed 的处理

如果结构化解析失败：

- `parse_status = Degraded | Invalid`
- 保留 `raw_fallback_text`
- `verification_status` 只能是 `NotRun` 或已有可信值
- 运行状态不能被误标为 fully successful structured run

### 12.5 强制 summary contract 的原因

没有固定 schema，`structured return` 很快会退化为口头习惯；
而多 Agent 协作依赖统一 contract 进行状态吸收、review、archive、artifact 读取和知识沉淀。

---

## 13. Dispatcher 与 Runner 抽象（合龙版）

### 13.1 当前问题

现状常见问题是：

- mock path 走统一 dispatcher
- 真实 provider path 走 provider-specific 分支
- 抽象存在，但真实执行未完全收敛到同一条链路

这必须修。

### 13.2 正式 trait

```rust
#[async_trait::async_trait]
pub trait AgentRunner: Send + Sync {
    fn provider(&self) -> Provider;
    fn capabilities(&self) -> RunnerCapabilities;

    async fn run(
        &self,
        ctx: RunExecutionContext,
    ) -> anyhow::Result<RunHandle>;

    async fn cancel(&self, handle: &RunHandle) -> anyhow::Result<()>;
}
```

```rust
pub struct RunExecutionContext {
    pub request: DispatchRequest,
    pub spec: AgentSpec,
    pub compiled_context: CompiledContext,
    pub workspace: WorkspaceContext,
    pub timeout_secs: u64,
    pub summary_schema_path: Option<std::path::PathBuf>,
    pub persistence: RunPersistenceContext,
}
```

### 13.3 Dispatcher 的职责

Dispatcher 只做以下事情：

1. spec 解析与校验
2. workflow gate 检查
3. context compile
4. workspace 准备
5. 选 runner
6. 启动执行
7. 等待 / 持久化 / 取消 / 回收

Dispatcher **不再**包含 provider-specific 命令拼装逻辑。

### 13.4 Runtime-managed spawn 深度

正式拍板：

- `max_runtime_depth` 默认 `1`
- 即主代理可以派发 child，但 child 不得继续 runtime-managed 派发 child
- provider-native 内部 subagent 行为视为 opaque，不纳入 runtime 句柄树

这样可以避免深层递归导致上下文爆炸、日志混乱和资源失控。

---

## 14. Provider 适配与能力矩阵

### 14.1 统一适配原则

Runner 负责做三件事：

1. 把统一 `AgentSpec` 映射为 provider 当前版本支持的 CLI 参数/临时配置
2. 将统一 sandbox / approval / workspace / summary contract 落到 provider 能理解的执行面
3. 处理 provider 不支持字段时的降级与告警

### 14.2 Provider capability matrix（实现版）

| 能力 | Codex | Claude | Gemini | Ollama |
|---|---:|---:|---:|---:|
| 真实 runner | Yes | Yes | Yes (experimental) | Reserved |
| 结构化输出原生 schema | Yes | Yes | Partial / no native schema guarantee | Depends |
| 额外目录挂载 | Yes | Yes | Yes | Depends |
| 独立上下文/子代理方向契合 | Yes | Yes | Yes | N/A |
| 原生项目记忆文件 | AGENTS.md | CLAUDE.md | GEMINI.md | Provider-specific |
| 可控 sandbox 参数 | Strong | Partial | Strong | Depends |
| approval 参数清晰 | Strong | Strong | Strong | Depends |
| 本项目支持层级 | Primary | Beta | Experimental | Reserved |

### 14.3 命令行映射原则（不要把示意语法写成永久事实）

文档不再写死“某一条 CLI 一定长这样”，统一改成：

> Runner 负责把统一 `AgentSpec` 映射到 provider 当前版本支持的 CLI 入口；
> 具体命令格式以该 provider 的当前 CLI 版本与本仓库适配器实现为准。

### 14.4 审批/权限/Sandbox 的统一映射（正式版本）

#### 内部抽象

- `SandboxMode::{ReadOnly, WorkspaceWrite, DangerFullAccess}`
- `ApprovalMode::{Default, OnRequest, Never}`

#### Codex 映射

- sandbox：直接映射
- approval：直接映射
- summary schema：优先原生 schema

#### Claude 映射

- approval：映射到 `permission-mode`
- sandbox：**不强行假设有对等 CLI flag**，由 runtime 的 workspace isolation + Claude 自身权限模式共同保障
- extra dirs：通过 `--add-dir`
- structured output：优先 `--json-schema`

#### Gemini 映射

- approval：映射到 `--approval-mode`
- sandbox：映射到 `--sandbox`
- extra dirs：通过 `--include-directories`
- structured output：prompt contract + 后置解析

### 14.5 参数映射策略（必须修正当前常见风险）

1. 任何 provider 参数值都必须基于当前官方文档和本地 probe 实测。
2. 不允许“我猜某个值应该能用”。
3. 参数映射失败要回到结构化错误，而不是静默使用未知值。
4. `doctor` 应明确展示当前 provider 版本、已识别能力和已验证 flag 组合。

---

## 15. 本地调试 CLI（直接可跑版本必须具备）

除了 `--mcp` 外，必须提供以下本地入口：

### 15.1 命令列表

```text
mcp-subagent doctor
mcp-subagent validate [--agents-dir <path>]
mcp-subagent list-agents [--json]
mcp-subagent run <agent> --task <text> [--plan PLAN.md] [--stage build]
mcp-subagent spawn <agent> --task <text> [--json]
mcp-subagent status <handle-id>
mcp-subagent cancel <handle-id>
mcp-subagent artifact <handle-id> [--kind summary|log|patch|json]
mcp-subagent --mcp
```

### 15.2 为什么必须有本地 CLI

- 不需要接入 MCP Host 就能调试 runtime
- 更利于验证 provider 参数映射
- 更利于 CI 和人工排错
- 能把“本地可跑”作为交付标准，而不是口头说法

### 15.3 本地运行的最小成功路径

```bash
cargo run -- doctor
cargo run -- validate
cargo run -- list-agents
cargo run -- run reviewer --task "检查当前仓库的 context compiler 设计" --stage review
cargo run -- --mcp
```

---

## 16. MCP Surface（正式版）

### 16.1 Tool 列表

保留现有 6 个底层 runtime tools：

- `list_agents`
- `run_agent`
- `spawn_agent`
- `get_agent_status`
- `cancel_agent`
- `read_agent_artifact`

### 16.2 `run_agent` 与 `spawn_agent` 的语义边界

- `run_agent`：同步便利封装，调用方等待最终结果
- `spawn_agent`：异步执行，返回句柄

### 16.3 spec 中的 `background_preference` 如何生效

不是由 tool 决定，而是：

- tool 决定调用方是否同步等待
- `background_preference` 决定 runner 是否优先采用 provider-native background 模式（若支持）

### 16.4 参数约束

`run_agent` / `spawn_agent` 只能接受：

- `task`
- `task_brief`
- `parent_summary`
- `selected_files`
- `stage`
- `plan_ref`

**明确禁止：** 传入 raw transcript。

### 16.5 未来上层业务 tools

本版本底层 runtime tools 保留；
后续可以在上层 MCP facade 或 host prompt 中封装具名能力，例如：

- `plan_work`
- `implement_change`
- `review_code`
- `summarize_repo`

但这不是当前内核收口的阻塞项。

---

## 17. 持久化布局

### 17.1 state 目录

默认：`.mcp-subagent/state/`

### 17.2 运行目录结构

```text
.mcp-subagent/state/runs/<handle-id>/
├── request.json
├── resolved-spec.json
├── compiled-context.md
├── status.json
├── summary.json
├── summary.raw.txt
├── events.ndjson
├── runner.stdout.log
├── runner.stderr.log
├── workspace.meta.json
└── artifacts/
    ├── index.json
    └── ...
```

### 17.3 workflow 目录

```text
PLAN.md
plans/
archive/
docs/plans/
docs/decisions/
```

### 17.4 artifact index

`artifacts/index.json` 应包含：

- artifact kind
- relative path
- mime / logical type
- producer (agent name)
- created_at
- description

---

## 18. 日志与观测

### 18.1 日志分层

- 进程级 tracing
- run 级事件流（`events.ndjson`）
- provider stdout/stderr 原文
- summary / parse 状态
- workspace / fallback notes

### 18.2 关键事件

至少记录：

- probe result
- validate result
- workflow gate pass/fail
- workspace policy resolved
- memory resolved / native passthrough decisions
- summary parse status
- cleanup result

### 18.3 Doctor 输出必须增加的内容

- provider 可执行文件是否存在
- provider 认证/登录状态（若可探测）
- CLI 版本
- 已识别能力矩阵
- 当前 repo 推荐 workspace 策略成本提示
- `PLAN.md` / project memory / archive 结构是否健康

---

## 19. 安全模型

### 19.1 安全边界

- runtime 负责 workspace 隔离、冲突策略、summary contract、artifact 审计
- provider CLI 负责其自身权限/审批/安全模型
- 两者叠加形成 defense-in-depth

### 19.2 默认安全姿态

- sandbox 默认 `ReadOnly`
- approval 默认 `OnRequest`
- write 任务默认隔离 workspace
- provider-native high-risk 模式不得在默认路径中静默开启

### 19.3 高危模式

`DangerFullAccess` 只允许在：

- 明确配置
- doctor 不报错
- workspace 为隔离副本或受控环境
- 持久化完整日志

时启用。

---

## 20. 与当前仓库相比的 P0 / P1 / P2 改造清单

这一章是给你和同事直接开工用的。

### 20.1 P0（必须先修）

#### P0-1. 让 `context_mode` 真正控制注入行为

**位置**：`runtime/context.rs`

目标：

- 将 `Isolated / SummaryOnly / SelectedFiles / ExpandedBrief` 落成真实分支逻辑
- 严格限制 parent summary / selected files / active plan 的注入集合
- 禁止“无论什么 mode 都全带”

交付标准：

- 单元测试覆盖四种模式
- 断言 raw transcript 不可能进入 compiled context

#### P0-2. 拆分 `src/mcp/server.rs`

**位置**：`mcp/server.rs`

目标：

- 至少拆成 `service.rs / tools.rs / state.rs / persistence.rs / dto.rs / artifacts.rs`
- 不再让一个文件同时承担 transport、tool entry、状态快照、artifact 读取、持久化

交付标准：

- 原功能不回退
- 主要 public API 不破坏外部调用

#### P0-3. 统一真实 runner 抽象

**位置**：`runtime/dispatcher.rs`, `runtime/runner.rs`, `runtime/runners/*`

目标：

- mock 与真实 runner 使用同一 `AgentRunner` trait
- 删除长期存在的 provider-specific dispatch 分叉

交付标准：

- `run_dispatch()` 只保留一条主链路
- provider-specific 逻辑只在 runner 内部

#### P0-4. 校准 provider 参数映射

**位置**：`runtime/runners/codex.rs`, `claude.rs`, `gemini.rs`, `probe/*`

目标：

- 所有 approval/sandbox/output 参数以当前官方文档 + 本地 probe 为准
- 不可用值立即报错

交付标准：

- `doctor` 输出已验证的 flag 组合
- 本地 smoke test 覆盖 Codex/Claude/Gemini 最小跑通路径

#### P0-5. 清理临时 workspace / worktree 生命周期

**位置**：`runtime/workspace.rs`, `runtime/cleanup.rs`

目标：

- temp_copy、git_worktree 都形成完整创建-使用-清理闭环

交付标准：

- 运行结束后无悬挂临时目录 / worktree
- 失败路径也有清理补偿

#### P0-6. 升级 summary contract

**位置**：`runtime/summary.rs`

目标：

- 从“sentinel + fallback”升级到 `SummaryEnvelope + parse_status + raw_fallback_text`
- `plan_refs`、`artifact index`、`touched_files`、`verification_status` 成为强字段

交付标准：

- Codex 优先 schema 输出
- Claude 优先 schema 输出
- Gemini 明确标记 post-validated / degraded

### 20.2 P1（紧接着做）

#### P1-1. 引入 WorkflowSpec

**位置**：`spec/workflow.rs`, `runtime/workflow/*`

目标：

- 增加工作流 gate、stage、plan、archive、knowledge capture

#### P1-2. `ActivePlan` memory source

**位置**：`runtime/memory.rs`, `runtime/workflow/active_plan.rs`

目标：

- 当前任务的 `PLAN.md` 自动进入 memory resolve
- 编译器生成 plan excerpt

#### P1-3. Stage-aware dispatcher

**位置**：`runtime/dispatcher.rs`

目标：

- 根据 stage + role 路由
- build/review 前要求有效 plan

#### P1-4. `WorkingDirPolicy::Auto`

**位置**：`spec/runtime_policy.rs`, `runtime/workspace.rs`, `doctor/report.rs`

目标：

- 读任务 in_place
- 写任务优先 git_worktree
- fallback temp_copy

### 20.3 P2（收口提升）

#### P2-1. 细粒度冲突策略

- 当前 repo 级 serialize 足够，但后续可按子目录或 predicted touched files 细化

#### P2-2. 上层业务 tools

- `plan_work` / `implement_change` / `review_code` / `summarize_repo`

#### P2-3. Plan archive 检索入口

- 归档后可被 future research / memory search 使用

#### P2-4. Ollama runner

- 只有在 probe + doctor + real runner + tests 完成后，才从 reserved 升级为 experimental

---

## 21. MVP 交付标准（本地可跑版本）

达到以下条件才算“本地可跑版本”：

### 21.1 必须满足

- `cargo run -- doctor` 正常输出可读报告
- `cargo run -- validate` 能验证 agents/workflow 配置
- `cargo run -- list-agents` 正常工作
- `cargo run -- run <agent> --task ...` 可在 Mock 和 Codex 上真实跑通
- `cargo run -- --mcp` 可提供稳定的 stdio MCP 服务
- `context_mode` 四种模式有测试
- `PLAN.md` gate 生效
- `SummaryEnvelope` 落盘成功
- temp/worktree 清理可验证

### 21.2 可以暂缓但要明确标注

- Claude Beta 的所有边角行为
- Gemini experimental 的全部高级能力
- Ollama 真 runner
- 细粒度并行锁

---

## 22. 示例配置

### 22.1 `agents/reviewer.agent.toml`

```toml
[core]
name = "reviewer"
description = "Review code for correctness, regressions, and missing tests."
provider = "codex"
model = "gpt-5-codex"
role = "ReviewerCorrectness"
instructions = "Review like an owner. Prioritize correctness, safety, behavior regressions, and missing tests."

[runtime]
context_mode = "SummaryOnly"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
sandbox = "ReadOnly"
approval = "OnRequest"
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
timeout_secs = 900
background_preference = "ForegroundPreferred"
spawn_policy = "WorkflowOnly"
isolation = "IsolatedWorkspace"

[workflow]
enabled = true
stages = ["Review", "Archive"]
max_runtime_depth = 1
```

### 22.2 `agents/coder.agent.toml`

```toml
[core]
name = "coder"
description = "Implement plan-aligned changes in isolated workspace."
provider = "codex"
model = "gpt-5-codex"
role = "Coder"
instructions = "Implement only the approved plan sections. Keep changes scoped and verifiable."

[runtime]
context_mode = "ExpandedBrief"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
sandbox = "WorkspaceWrite"
approval = "OnRequest"
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
timeout_secs = 1800
background_preference = "ForegroundPreferred"
spawn_policy = "WorkflowOnly"
isolation = "IsolatedWorkspace"

[workflow]
enabled = true
stages = ["Build"]
max_runtime_depth = 1
```

### 22.3 `workflows/default.workflow.toml`

```toml
enabled = true
stages = ["Research", "Plan", "Build", "Review", "Archive"]
max_runtime_depth = 1

[require_plan_when]
require_plan_if_touched_files_ge = 5
require_plan_if_cross_module = true
require_plan_if_parallel_agents = true
require_plan_if_new_interface = true
require_plan_if_migration = true
require_plan_if_human_approval_point = true
require_plan_if_estimated_runtime_minutes_ge = 15

[active_plan]
enabled = true
prefer_root_plan = true

[review_policy]
require_correctness_review = true
require_style_review = false
allow_same_provider_dual_review = true
prefer_cross_provider_review = true

[knowledge_capture]
trigger_if_touched_files_gt = 3
trigger_if_new_config = true
trigger_if_behavior_change = true
trigger_if_non_obvious_bugfix = true
write_decision_note = true
update_project_memory = false

[archive_policy]
enabled = true
archive_dir = "docs/plans"
write_final_summary = true
write_metadata_index = true
```

---

## 23. 测试策略

### 23.1 单元测试

- context mode 分支行为
- memory resolve 与去重
- native memory passthrough / fallback inline
- summary parse / degrade / invalid
- workflow gate 判断
- workspace policy `Auto` 解析

### 23.2 集成测试

- Mock runner 全链路
- state 持久化
- artifact 读取
- MCP tools 基本调用
- `PLAN.md` gate 和 active plan 注入

### 23.3 e2e / smoke test

优先顺序：

1. Codex
2. Claude
3. Gemini

要求：

- 本机已安装 CLI
- doctor 检测通过
- 最小 agent 配置
- 简单只读任务 / 简单写任务 各一条

---

## 24. 迁移建议（从当前仓库到本版设计）

### 第一周：收口内核

- 修 `context_mode`
- 拆 `mcp/server.rs`
- 合龙 runner trait
- 校准 provider flags
- 清理 workspace 生命周期

### 第二周：加工作流层

- `WorkflowSpec`
- `ActivePlan`
- stage-aware dispatcher
- plan gate
- archive / knowledge capture

### 第三周：打磨首个真实稳定路径

- 优先 Codex
- 完善 summary schema 和 e2e
- 明确本地运行文档

### 第四周：扩 provider

- Claude Beta
- Gemini experimental
- 视情况开始 Ollama runner

---

## 25. 最终结论

这版文档的立场非常明确：

- **不推翻现有仓库**，因为骨架已经搭出来了。
- **先收口再扩展**，因为现在最大的风险不是方向错，而是 schema 与执行行为脱节，以及复杂度失控。
- **工作流层必须进入系统本体**，因为多 Agent 协作的关键不是“能调度”，而是“围绕同一工件协作”。
- **本版本的成功标准不是功能罗列，而是本地真的能跑**：`doctor`、`validate`、`run`、`--mcp`、`PLAN.md` gate、`SummaryEnvelope`、workspace cleanup 都必须成立。

一句话拍板：

> `mcp-subagent-rs` 的下一阶段，不应该继续盲目加 provider 或加字段；
> 应该先把 **Context / Workflow / Workspace / Summary / Runner 抽象** 这五块打实，
> 然后以 Codex 为主路径做出一个稳定的本地可跑版本，再扩其他 provider。

---

## 26. 参考资料（供实现时核对）

以下链接建议在实现和 flag 校准时随代码一并核对：

- Claude Code CLI reference
- Claude Code permissions
- Claude Code subagents
- Claude Code memory / CLAUDE.md
- OpenAI Codex CLI reference
- OpenAI Codex subagents
- OpenAI Codex AGENTS.md guide
- Gemini CLI reference
- Gemini CLI subagents (experimental)
- Gemini CLI GEMINI.md / memory docs
- rmcp docs.rs / README

> 版本说明：实现时一切以“当前 provider CLI 文档 + 本机 doctor/probe 实测”双重确认后为准。

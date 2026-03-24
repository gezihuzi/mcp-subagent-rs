
# mcp-subagent 技术设计文档 v0.5（开发基线版）

- **项目名 / crate**：`mcp-subagent`
- **仓库名**：`mcp-subagent-rs`
- **文档状态**：开发基线版（可交付工程实现；未冻结为长期协议）
- **最后修订**：2026-03-24
- **目标读者**：Rust 开发、架构评审、平台工程、AI 工具链负责人

---

## 0. 文档结论与决策摘要

这份文档将 `mcp-subagent` 定义为一个**单一 Rust 二进制**：对外暴露标准 **MCP Server**，对内实现**多 provider 的本地 CLI 调度 runtime**。  
它**不直接集成厂商 API**，而是统一通过本机已安装的 `claude` / `codex` / `gemini` 等 CLI 调用模型；是否联网、如何认证，取决于底层 provider/CLI，而不是 runtime 本身。  

### 0.1 最终结论

`mcp-subagent` 的主架构采用以下固定方案：

1. **单一 Rust 二进制 + MCP Server 外壳 + 内嵌 runtime/dispatcher**
2. **统一 AgentSpec，但拆为 Core / RuntimePolicy / ProviderOverrides 三层**
3. **默认隔离上下文 + 显式注入 + 结构化回传**
4. **禁止 raw parent transcript 透传**
5. **runner 允许在运行时生成临时 vendor config / agent file**
6. **provider 能力差异通过 capability matrix 与降级策略处理**
7. **读写并发通过 working_dir_policy + file_conflict_policy 管控**
8. **MVP 以 stdio MCP transport 为先；HTTP 为后续扩展**
9. **Codex / Gemini 先落地，Claude 随后接入；Ollama/llama.cpp 作为未来纯本地模型扩展**

### 0.2 新增的关键工程修正

除前几轮讨论外，本版文档额外补了一条非常重要的实现约束：

> **避免“双重加载 provider 原生记忆文件”**  
> Claude/Codex/Gemini 本身都会在各自 CLI 中自动发现并读取部分项目记忆文件。  
> 因此 runtime **不能**一边让 provider 自动读取，一边又把同样内容重新内联到编译后的 prompt 中，否则会造成重复指令、token 浪费、优先级漂移和行为不稳定。

`mcp-subagent` 的默认策略因此改为：

- provider 原生记忆文件优先走 provider 自己的原生加载机制；
- runtime 自己额外维护一个 provider-neutral 的 `PROJECT.md`；
- ContextCompiler 默认只显式编译：
  - `PROJECT.md`
  - `parent_summary`
  - `task_brief`
  - `selected_files`
  - 显式声明的额外 memory sources
- 若必须跨 provider 转译某个记忆源，runner 必须执行**去重与来源标注**。

### 0.3 规范性关键词

本文中的 **MUST / SHOULD / MAY** 采用如下含义：

- **MUST**：硬约束；实现不得违背
- **SHOULD**：强建议；除非有明确理由，否则应遵守
- **MAY**：可选项；按实现复杂度与优先级决定

---

## 1. 外部事实基线（已核对官方文档）

以下事实决定了本设计的边界条件：

1. **Claude Code subagents** 有独立的 context window、独立权限与自定义系统提示；自定义 subagent 使用 **Markdown + YAML frontmatter** 定义，并支持前后台运行、权限模式、skills、hooks、inline MCP server 等。  
2. **Codex subagents** 的核心价值之一就是减少 context pollution / context rot；Codex 的 subagent workflow 需要显式触发；自定义 agent 使用 **TOML**，并支持 `model`、`model_reasoning_effort`、`sandbox_mode`、`mcp_servers`、`skills.config` 等字段。  
3. **Gemini CLI subagents** 目前仍是 **experimental**；自定义 subagent 使用 **Markdown + YAML frontmatter**；支持 `@agent` 显式委派；主 agent 与 subagent 之间是独立上下文循环。  
4. **Claude / Codex / Gemini 都不是“天然离线本地模型”**。如果通过这些厂商 CLI 调用模型，runtime 虽然不直接接 API，但底层 provider 仍可能要求登录、联网或依赖其云服务。  
5. **rmcp 1.2.0** 是当前官方 Rust SDK，支持 server/macros 以及 stdio / streamable HTTP 等 transport feature。

因此，本文档明确采用：

- **“本机 CLI 调用” ≠ “完全离线 / 本地推理”**
- 真正的离线本地模型应通过未来的 `OllamaRunner`、`LlamaCppRunner` 等实现接入
- `rmcp` 只以**最小可用 feature 集**起步，不预先脑补完整功能矩阵

---

## 2. 项目目标与非目标

## 2.1 目标

`mcp-subagent` 的目标是：

- 向 Claude Desktop、Cursor、Codex、其他 MCP Host 提供一个标准 **MCP Server**
- 在本机统一调度多个 provider 的 CLI agent/subagent 能力
- 让用户通过一份统一的 `*.agent.toml` 描述 agent
- 在不复制主会话原始 transcript 的前提下，完成任务委派、上下文编译、执行与结果回收
- 让多 LLM 可以在一台设备上通过**共享文件系统 + 结构化摘要**协作
- 为后续接入 Ollama / llama.cpp / DeepSeek CLI 等保留扩展点

## 2.2 非目标

以下内容不属于本项目的主路径：

- **不**在 runtime 内直接对接厂商 API
- **不**承诺 Claude / Codex / Gemini 的完全离线运行
- **不**以导出并长期维护 provider 原生配置为主路径
- **不**实现全量共享聊天历史或共享长上下文记忆库
- **不**做分布式 agent 集群或远程 worker 编排
- **不**在 v0.x 阶段实现复杂 GUI
- **不**以 binary artifact 流式回传为 MVP 目标

---

## 3. 设计原则

### 3.1 上下文原则

1. **默认隔离**
2. **禁止 raw transcript**
3. **共享稳定事实，不共享噪声**
4. **spawn 时只给 briefing / summary / selected context**
5. **结束时只收结构化 summary + artifacts**
6. **长期记忆更新必须显式**

### 3.2 集成原则

1. **CLI-first，不写死语法**
2. **provider 差异收敛到 adapter 层**
3. **MCP surface 尽量小而稳定**
4. **运行时 bridge 可以临时生成 vendor config**
5. **能力不足时优雅降级，而不是伪支持**

### 3.3 工程原则

1. **单一二进制**
2. **tokio 异步并发**
3. **tracing 可观测**
4. **结构化状态与错误**
5. **跨 provider 的最小共同抽象**
6. **写操作默认隔离**
7. **先最小可运行，再逐步补齐高级能力**

---

## 4. 系统架构总览

```text
mcp-subagent (single binary)
├── CLI / bootstrap
│   ├── mcp-subagent --mcp        # stdio MCP server
│   ├── mcp-subagent --http       # optional, future
│   ├── mcp-subagent doctor       # local diagnostics
│   └── mcp-subagent validate     # spec/config validation
│
├── MCP Server layer (rmcp)
│   ├── tool routing
│   ├── request validation
│   └── response normalization
│
└── Agent Runtime layer
    ├── Agent registry / spec loader
    ├── Provider probe
    ├── ContextCompiler
    ├── Dispatcher / scheduler
    ├── Workspace manager
    ├── State store / log store / artifact store
    ├── Runner trait
    │   ├── ClaudeRunner
    │   ├── CodexRunner
    │   ├── GeminiRunner
    │   └── OllamaRunner (future)
    └── Summary parser / result normalizer
```

### 4.1 双层职责划分

#### MCP Server 层职责

- 暴露稳定的工具接口
- 将 MCP tool call 转换为 runtime 请求
- 对参数做 schema 级校验
- 将运行状态、日志、artifact 元数据回传给 host

#### Runtime 层职责

- 加载 agent spec
- 探测 provider 可用性与版本
- 编译上下文
- 选择并启动对应 runner
- 管理并发、状态、取消、超时、日志、artifact
- 解析结构化 summary
- 执行去重、降级与错误归一化

---

## 5. 仓库结构建议（mcp-subagent-rs）

```text
mcp-subagent-rs/
├── Cargo.toml
├── README.md
├── LICENSE
├── docs/
│   ├── architecture.md
│   ├── provider-matrix.md
│   ├── run-lifecycle.md
│   └── examples/
├── agents/
│   ├── reviewer.agent.toml
│   ├── implementer.agent.toml
│   ├── planner.agent.toml
│   └── investigator.agent.toml
├── src/
│   ├── main.rs
│   ├── cli.rs
│   ├── error.rs
│   ├── config.rs
│   ├── types.rs
│   ├── mcp/
│   │   ├── mod.rs
│   │   ├── server.rs
│   │   ├── tools.rs
│   │   └── dto.rs
│   ├── runtime/
│   │   ├── mod.rs
│   │   ├── dispatcher.rs
│   │   ├── context.rs
│   │   ├── summary.rs
│   │   ├── workspace.rs
│   │   ├── artifacts.rs
│   │   ├── logs.rs
│   │   ├── registry.rs
│   │   ├── state.rs
│   │   ├── cancellation.rs
│   │   └── capability.rs
│   ├── spec/
│   │   ├── mod.rs
│   │   ├── core.rs
│   │   ├── runtime_policy.rs
│   │   ├── provider_overrides.rs
│   │   ├── serde.rs
│   │   └── validate.rs
│   ├── probe/
│   │   ├── mod.rs
│   │   ├── claude.rs
│   │   ├── codex.rs
│   │   └── gemini.rs
│   └── runners/
│       ├── mod.rs
│       ├── trait.rs
│       ├── common.rs
│       ├── claude.rs
│       ├── codex.rs
│       ├── gemini.rs
│       └── ollama.rs
└── tests/
    ├── spec_validation.rs
    ├── context_compiler.rs
    ├── mock_runner.rs
    ├── mcp_e2e.rs
    └── fixtures/
```

---

## 6. 二进制接口设计

## 6.1 子命令

### `mcp-subagent --mcp`

- 启动 stdio MCP server
- MVP 默认入口
- 面向 Claude Desktop / Cursor / 其他 MCP host

### `mcp-subagent --http`

- 启动 HTTP MCP server
- 默认不作为 MVP 主入口
- 仅在 rmcp HTTP transport 最小 demo 验证通过后启用

### `mcp-subagent doctor`

用于本地诊断，输出：

- 可执行文件探测结果（claude / codex / gemini）
- 版本信息
- provider capability probe 结果
- state dir / config dir / agents dir
- 是否发现明显认证问题（尽力而为，不要求 100% 准确）

### `mcp-subagent validate`

用于 CI 或开发自检：

- 校验 `*.agent.toml`
- 校验字段合法性
- 校验 provider override 仅出现在对应 provider
- 校验 memory/source 路径
- 校验 summary contract 模板是否完整

---

## 7. AgentSpec 规范

## 7.1 顶层结构

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpec {
    pub core: AgentSpecCore,
    pub runtime: RuntimePolicy,
    pub provider_overrides: ProviderOverrides,
}
```

### 7.2 AgentSpecCore（跨 provider 稳定字段）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSpecCore {
    pub name: String,
    pub description: String,
    pub provider: Provider,
    pub model: Option<String>,
    pub instructions: String,

    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub skills: Vec<String>,

    pub tags: Vec<String>,
    pub metadata: HashMap<String, serde_json::Value>,
}
```

#### 说明

- `name`：逻辑 agent 名；MCP 与 runtime 使用它，不依赖文件名
- `description`：供主代理或调度器选择 agent 时参考
- `provider`：目标 provider
- `model`：可选；为空时允许 runner 使用 provider 默认值
- `instructions`：统一的人类可读任务规范
- `allowed_tools` / `disallowed_tools`：逻辑级抽象，不要求与 provider 原生命名一字不差
- `skills`：runtime 级预加载能力描述；是否映射为 provider 原生 skills 取决于 runner

### 7.3 RuntimePolicy（运行策略）

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimePolicy {
    pub context_mode: ContextMode,
    pub memory_sources: Vec<MemorySource>,

    pub working_dir_policy: WorkingDirPolicy,
    pub file_conflict_policy: FileConflictPolicy,

    pub sandbox: SandboxPolicy,
    pub approval: ApprovalPolicy,

    pub max_turns: Option<u32>,
    pub timeout_secs: u64,
    pub background_preference: BackgroundPreference,

    pub spawn_policy: SpawnPolicy,
    pub artifact_policy: ArtifactPolicy,
    pub retry_policy: RetryPolicy,
}
```

### 7.4 ProviderOverrides（厂商特有字段）

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderOverrides {
    pub claude: Option<ClaudeOverrides>,
    pub codex: Option<CodexOverrides>,
    pub gemini: Option<GeminiOverrides>,
}
```

#### 设计约束

- 只允许当前 `core.provider` 对应的 override 生效
- 非目标 provider 的 override **SHOULD** 在校验阶段直接报错，而不是静默忽略
- `serde(deny_unknown_fields)` **SHOULD** 用于减少拼写错误

### 7.5 关键枚举

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum Provider {
    Claude,
    Codex,
    Gemini,
    Ollama,   // future
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum ContextMode {
    Isolated,
    SummaryOnly,
    SelectedFiles(Vec<String>),
    ExpandedBrief,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MemorySource {
    AutoProjectMemory,
    File(String),
    Glob(String),
    Inline(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum WorkingDirPolicy {
    InPlace,
    TempCopy,
    GitWorktree,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum FileConflictPolicy {
    Deny,
    Serialize,
    AllowWithMergeReview,
}
```

### 7.6 默认值

建议默认值：

- `context_mode = Isolated`
- `memory_sources = [AutoProjectMemory]`
- `working_dir_policy = TempCopy`
- `file_conflict_policy = Serialize`
- `sandbox = ReadOnly`
- `approval = ProviderDefault`
- `timeout_secs = 900`
- `background_preference = PreferForeground`
- `spawn_policy = Sync`
- `artifact_policy.emit_summary_json = true`

> **说明**：  
> 早期讨论里常把写代码 agent 直接放到 `in_place`。本版不建议这样做。  
> 对写能力 agent，默认使用 `TempCopy` 或 `GitWorktree` 更稳。

---

## 8. 项目记忆与上下文模型

## 8.1 核心规则

### RULE-CTX-001（MUST）

`mcp-subagent` **MUST NOT** 将父 agent 的原始 transcript 直接转发给子 agent。

### RULE-CTX-002（MUST）

子 agent 只允许收到以下显式上下文源：

- 编译后的 `task_brief`
- 结构化 `parent_summary`
- `selected_files`
- 显式声明的 `memory_sources`
- runtime 拥有的共享项目记忆（例如 `PROJECT.md`）

### RULE-CTX-003（MUST）

子 agent 执行结束后，回传结果必须优先走**固定结构化 contract**，而不是自由文本。

### RULE-CTX-004（MUST）

若 provider CLI 会自动加载其原生记忆文件，则 runner **MUST** 避免把同一内容再次内联到 prompt 中。

## 8.2 provider 原生记忆文件

`mcp-subagent` 承认以下 provider 原生记忆文件：

- Claude：`CLAUDE.md` / `.claude/CLAUDE.md`
- Codex：`AGENTS.md` / `AGENTS.override.md`
- Gemini：`GEMINI.md`

此外，runtime 自己引入一个**provider-neutral** 文件：

- `PROJECT.md`

### 8.2.1 设计建议

- **`PROJECT.md`**：放跨 provider 的稳定项目事实  
  如架构约束、术语、仓库边界、编码政策、测试入口、交付要求
- **`CLAUDE.md` / `AGENTS.md` / `GEMINI.md`**：放 provider-specific 的提示差异或原生集成指令
- `PROJECT.md` 可以由 runtime 主动编译注入
- provider 原生文件优先让对应 CLI 自己加载

## 8.3 AutoProjectMemory 解析策略

`AutoProjectMemory` 的默认行为定义为：

1. 先找 runtime 共享记忆：
   - `./PROJECT.md`
   - `./.mcp-subagent/PROJECT.md`
   - 用户级 `PROJECT.md`（可配置）
2. 再识别当前 provider 的原生记忆文件
3. 对原生记忆文件只记录**元信息与路径**
4. 由 runner 在执行阶段决定：
   - **NativePassThrough**：完全交给 provider CLI 自己加载
   - **InlineSummary**：抽取成摘要注入
   - **RawInline**：显式内联（仅当 provider 无原生加载或已确认不会重复）

### 8.3.1 MVP 默认

MVP 中：

- `PROJECT.md`：runtime 主动编译注入
- `CLAUDE.md` / `AGENTS.md` / `GEMINI.md`：默认采用 **NativePassThrough**
- 显式 `File(...)` / `Glob(...)`：由 ContextCompiler 摘要后注入
- 若用户强制要求完整文件注入，必须在 summary/log 中记录来源

---

## 9. ContextCompiler 设计

## 9.1 接口

```rust
#[async_trait]
pub trait ContextCompiler: Send + Sync {
    async fn compile(
        &self,
        spec: &AgentSpec,
        request: &RunRequest,
        memory: ResolvedMemory,
    ) -> Result<CompiledContext>;

    async fn parse_summary(
        &self,
        raw_stdout: &str,
        raw_stderr: &str,
    ) -> Result<StructuredSummary>;
}
```

## 9.2 输入

`RunRequest` 至少包含：

- `task: String`
- `task_brief: Option<String>`
- `parent_summary: Option<String>`
- `selected_files: Vec<SelectedFile>`
- `working_dir: PathBuf`
- `run_mode: RunMode`

## 9.3 输出

```rust
pub struct CompiledContext {
    pub system_prefix: String,
    pub injected_prompt: String,
    pub source_manifest: Vec<ContextSourceRef>,
}
```

`source_manifest` 用于：

- 调试去重问题
- 记录哪些文件通过 native pass-through 处理
- 方便开发时定位 summary 偏差

## 9.4 固定编译模板

编译产物建议包含固定段落：

1. `ROLE`
2. `TASK`
3. `OBJECTIVE`
4. `CONSTRAINTS`
5. `ACCEPTANCE CRITERIA`
6. `SELECTED CONTEXT`
7. `RESPONSE CONTRACT`
8. `OUTPUT SENTINELS`

### 9.4.1 RESPONSE CONTRACT（MUST）

每个子 agent 最终输出**必须包含**一个 machine-readable JSON 块，使用固定哨兵包裹：

```text
<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>
{ ... valid json ... }
<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>
```

自由文本可以存在，但解析器只信任哨兵内的 JSON。

### 9.4.2 解析降级

若未找到合法 JSON：

1. 尝试从 YAML/JSON 片段修复
2. 若修复失败，生成 `degraded summary`
3. 将 `verification_status = ParseFailed`
4. 标记本次 run 为 `CompletedWithDegradedSummary`

---

## 10. StructuredSummary 固定 contract

## 10.1 结构定义

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredSummary {
    pub summary: String,
    pub key_findings: Vec<String>,
    pub artifacts: Vec<ArtifactRef>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
    pub exit_code: i32,
    pub verification_status: VerificationStatus,
    pub touched_files: Vec<String>,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub path: PathBuf,
    pub kind: ArtifactKind,
    pub description: String,
    pub media_type: Option<String>,
}
```

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum VerificationStatus {
    NotRun,
    Passed,
    Failed,
    Partial,
    ParseFailed,
}
```

## 10.2 设计说明

相比只返回 `Vec<PathBuf>`，`ArtifactRef` 更适合：

- MCP 层展示
- artifact 类型筛选
- 后续 HTTP/API 化
- 主 agent 按 kind 选择读取策略

## 10.3 Summary contract 约束

### MUST

- `summary` 必须存在
- `exit_code` 必须存在
- `verification_status` 必须存在
- `artifacts` 中的 `path` 必须是可解析路径
- `touched_files` 应尽量列出修改过或重点关注过的文件

### SHOULD

- `key_findings` 不少于 1 条
- `open_questions` 对未完成情况给出原因
- `next_steps` 给出后续可执行动作

---

## 11. Dispatcher 与运行生命周期

## 11.1 固定生命周期

```text
RECEIVED
-> VALIDATING
-> PROBING_PROVIDER
-> PREPARING_WORKSPACE
-> RESOLVING_MEMORY
-> COMPILING_CONTEXT
-> LAUNCHING
-> RUNNING
-> COLLECTING
-> PARSING_SUMMARY
-> FINALIZING
-> SUCCEEDED | FAILED | CANCELLED | TIMED_OUT
```

## 11.2 详细步骤

1. **校验请求**
   - agent 是否存在
   - provider 是否支持
   - 运行策略是否合法
2. **probe provider**
   - 可执行文件是否存在
   - 版本信息
   - 是否支持目标桥接模式
3. **准备工作目录**
   - `in_place` / `temp_copy` / `git_worktree`
4. **解析记忆**
   - 发现 `PROJECT.md`
   - 识别 provider 原生记忆文件
   - 执行 dedup planning
5. **编译上下文**
6. **启动 runner**
7. **持续采集 stdout/stderr / logs**
8. **超时或取消处理**
9. **解析 summary**
10. **写入状态与 artifact 索引**
11. **返回 MCP 响应**

## 11.3 句柄与状态

每个 run 拥有：

- `handle_id`：建议 UUID v7
- `created_at`
- `updated_at`
- `status`
- `provider`
- `agent_name`
- `workspace_path`
- `log_paths`
- `artifact_index`
- `summary_path`

---

## 12. Working Directory 策略

## 12.1 `in_place`

直接在原仓库运行。

**适用场景**：

- 只读 agent
- 明确串行写入
- 用户主动要求

**风险**：

- 与人类开发并发修改冲突
- 与其他 agent 并发写冲突
- provider 原生文件写入影响主仓库

## 12.2 `temp_copy`

复制到临时目录执行。

**适用场景**：

- 默认方案
- 小中型仓库
- 写入隔离优先

**优点**：

- 简单
- 隔离好
- 不依赖 git

**缺点**：

- 大仓库复制成本高
- 与真实分支状态可能漂移

## 12.3 `git_worktree`

使用 git worktree 创建隔离工作树。

**适用场景**：

- git 仓库
- 写代码 agent
- 希望保留真实版本控制语义

**建议**：

- 对写能力 agent，`git_worktree` 是首选
- 若创建失败，则退回 `temp_copy`
- 不支持 git 的目录退回 `temp_copy`

### 默认规则（SHOULD）

- 只读 agent：`in_place`
- 写能力 agent：`git_worktree`，失败则 `temp_copy`

---

## 13. 文件冲突策略

## 13.1 `deny`

检测到潜在冲突时直接拒绝启动。

## 13.2 `serialize`

同一仓库的写 agent 串行执行。

## 13.3 `allow_with_merge_review`

允许并发写，但结果必须以 patch/diff 或 worktree 形式回收，由主 agent 或人类做 merge review。

### MVP 默认

- `file_conflict_policy = Serialize`

### 检测依据

冲突检测可综合以下来源：

- 运行前的路径声明
- `touched_files`
- 工作目录 diff
- artifact 中的 patch 文件

---

## 14. Runner Trait 与 provider 适配层

## 14.1 统一接口

```rust
#[async_trait]
pub trait AgentRunner: Send + Sync {
    fn provider(&self) -> Provider;

    async fn probe(&self) -> Result<ProviderProbe>;

    async fn prepare(
        &self,
        spec: &AgentSpec,
        ctx: &CompiledContext,
        workspace: &PreparedWorkspace,
    ) -> Result<PreparedRun>;

    async fn launch(&self, prepared: PreparedRun) -> Result<RunHandle>;

    async fn poll(&self, handle: &RunHandle) -> Result<RunSnapshot>;

    async fn cancel(&self, handle: &RunHandle) -> Result<()>;

    async fn collect(&self, handle: RunHandle) -> Result<CollectedRun>;
}
```

## 14.2 为什么拆成 prepare / launch / collect

这样可以清晰分离：

- 配置桥接
- 进程启动
- 生命周期管理
- 收尾清理

也便于测试：

- `prepare()` 可独立 snapshot 测试
- `collect()` 可对 mock 输出做解析测试

## 14.3 CLI 语法约束

**MUST**：

- 设计文档中不把任何单个 provider 的 CLI 命令形态写死成正式协议
- adapter 负责把统一 spec 映射到“当前版本已验证”的 CLI 入口
- 具体语法以 probe + 版本锁 + 集成测试结果为准

### 原因

Claude / Codex / Gemini 的 CLI 都在持续演进。  
设计文档若把某条命令行样式写成“已经确认的长期接口”，会直接降低实现可靠性。

---

## 15. Provider 适配策略

## 15.1 ClaudeRunner

### 已知能力边界

- 自定义 subagent：Markdown + YAML frontmatter
- 支持前后台 subagent
- 支持工具限制、权限模式、skills、hooks
- 支持 inline MCP server
- 有 `CLAUDE.md` 记忆系统

### 设计策略

- 优先复用 Claude 原生 subagent 文件能力
- 临时文件作为运行时 bridge，结束后清理
- 默认不重复内联 `CLAUDE.md` 内容
- 背景执行能力由 Claude 原生能力配合 runtime handle 管理

## 15.2 CodexRunner

### 已知能力边界

- 自定义 agent：TOML
- subagent workflow 需要显式触发
- 自定义 agent 可包含 `model`、`model_reasoning_effort`、`sandbox_mode`、`mcp_servers`、`skills.config`
- `AGENTS.md` 是原生项目指导机制
- 本地 CLI 首次运行要求登录或 API key

### 设计策略

- 将统一 spec 映射到临时 TOML agent file
- 优先用非交互、可编排的 CLI 入口
- 默认不重复内联 `AGENTS.md`
- `model_reasoning_effort` 只在 Codex override 中暴露

## 15.3 GeminiRunner

### 已知能力边界

- subagents 仍是 experimental
- 自定义 agent：Markdown + YAML frontmatter
- 支持 `@agent` 显式指派
- subagent 独立 context loop
- `GEMINI.md` 为层级 context 文件

### 设计策略

- 文档、代码、日志中都要显式标注 experimental
- 默认关闭任何“递归再委派”假设
- 只做最小必要桥接
- 当实验特性版本不兼容时，优雅降级为普通单 agent 调用或直接报不支持

## 15.4 OllamaRunner（未来）

### 目标

- 纯本地模型 / 可离线运行
- 不依赖厂商云认证
- 与前述 vendor CLI 分层清晰

---

## 16. Provider Capability Matrix（用于适配，不作为营销描述）

> 本表表示 `mcp-subagent` 设计时采用的**适配基线**，而不是对任意未来版本 provider 的绝对承诺。

| 维度 | Claude | Codex | Gemini | Ollama（未来） |
|---|---|---|---|---|
| 自定义 agent 格式 | Markdown + YAML | TOML | Markdown + YAML | runtime 自定义 |
| provider 原生项目记忆 | `CLAUDE.md` | `AGENTS.md` | `GEMINI.md` | 无统一标准 |
| 自定义 subagent 成熟度 | 高 | 高 | Experimental | runtime 定义 |
| 显式 subagent/agent 委派 | 有 | 有 | 有（`@agent`） | runtime 定义 |
| 原生背景运行能力 | 有 | 不依赖原生背景语义 | 不依赖原生背景语义 | runtime 决定 |
| 原生 MCP 集成 | 有 | 有 | 部分能力存在但不作为 MVP 假设 | runtime 决定 |
| 原生登录/认证要求 | 有 | 有 | 有 | 取决于本地模型 |
| 默认是否可视为离线 | 否 | 否 | 否 | 可做到 |

### 16.1 降级规则

- 若某 provider 缺少背景执行能力，runtime 仍可通过异步进程句柄实现**宿主侧异步**
- 若某 provider 不支持某个字段，runner 必须：
  1. 忽略并告警，或
  2. 在校验期报错
- Gemini 若检测到不兼容 experimental 版本：
  - `run_agent` 返回 capability error
  - `list_agents` 标注 unavailable

---

## 17. Provider Probe 与版本锁

## 17.1 为什么需要 probe

因为本文明确**不把 CLI 语法写死**，所以 runtime 必须在启动时探测：

- 二进制是否存在
- 版本号
- 基本 help / feature 是否可用
- 是否支持目标 bridge 策略
- 当前 adapter 是否已验证该版本范围

## 17.2 ProviderProbe 结构

```rust
pub struct ProviderProbe {
    pub provider: Provider,
    pub executable: PathBuf,
    pub version: String,
    pub status: ProbeStatus,
    pub capabilities: ProviderCapabilities,
    pub notes: Vec<String>,
}
```

### ProbeStatus

- `Ready`
- `MissingBinary`
- `UnsupportedVersion`
- `NeedsAuthentication`
- `ExperimentalUnavailable`
- `ProbeFailed`

## 17.3 版本策略

- 文档中只锁 **rmcp 1.2.0**
- provider CLI 采用“版本范围 + 测试基线”的方式记录
- 每个 runner 都应维护 `SUPPORTED_VERSION_RANGES`

---

## 18. MCP 工具面设计

## 18.1 MVP 暴露工具

```rust
list_agents()
run_agent(...)
spawn_agent(...)
get_agent_status(...)
cancel_agent(...)
read_agent_artifact(...)
```

## 18.2 工具定义

### `list_agents`

返回：

- agent 名称
- 描述
- provider
- 可用性
- 主要运行策略摘要
- capability 降级提示

### `run_agent`

同步运行，等待完成后返回：

- `handle_id`
- `status`
- `structured_summary`
- `artifact_index`

### `spawn_agent`

异步启动，立刻返回 `handle_id`

### `get_agent_status`

返回：

- 当前状态
- 进度摘要
- 最近更新时间
- 日志路径/摘要
- 若完成则附带 summary 元数据

### `cancel_agent`

取消运行中的任务

### `read_agent_artifact`

MVP 只保证读取 UTF-8 文本 artifact。  
二进制 artifact 先返回元数据与路径，不承诺直接经 MCP 回传原始二进制。

## 18.3 参数约束

`run_agent` / `spawn_agent` 只允许传入：

- `task`
- `task_brief`
- `parent_summary`
- `selected_files`
- `working_dir`
- `agent_name`

**不允许**直接传原始 transcript。

---

## 19. 状态、日志与产物

## 19.1 状态目录

建议默认状态目录：

- Linux/macOS/WSL：`~/.local/share/mcp-subagent/`
- Windows：系统 app data 目录下的 `mcp-subagent/`

目录结构建议：

```text
state/
└── runs/
    └── <handle_id>/
        ├── run.json
        ├── stdout.log
        ├── stderr.log
        ├── summary.json
        ├── artifacts/
        └── temp/
```

## 19.2 run.json

记录：

- request snapshot
- spec snapshot
- probe result
- workspace metadata
- timestamps
- final status
- error details

## 19.3 日志

- 使用 `tracing`
- 每个 run 独立日志文件
- server 自身另有全局日志
- 支持 `RUST_LOG` / `--log-level`

## 19.4 Artifact policy

MVP 推荐支持：

- `summary.json`
- `report.md`
- `report.json`
- `patch.diff`
- `stdout.txt`
- `stderr.txt`

---

## 20. 安全与权限模型

## 20.1 设计边界

`mcp-subagent` 控制的是：

- 如何组织上下文
- 如何准备工作目录
- 如何调用 provider CLI
- 如何处理本地文件读写策略

`mcp-subagent` **不能替 provider 本身“关掉网络”**。  
如果底层 provider 需要联网认证或远程推理，runtime 不应伪装成离线系统。

## 20.2 抽象权限模型

```rust
pub enum SandboxPolicy {
    ReadOnly,
    WorkspaceWrite,
    FullAccess,
}

pub enum ApprovalPolicy {
    ProviderDefault,
    Ask,
    AutoAcceptEdits,
    DenyByDefault,
}
```

### 约束

- `FullAccess` 必须显式声明
- `ReadOnly` agent 不得使用写工作目录策略
- background write agent 默认不得使用 `FullAccess`

## 20.3 高风险目录保护

即便 provider 支持跳过权限，runtime 也 **SHOULD** 额外保护：

- `.git/`
- `.claude/`
- `.codex/`
- `.gemini/`
- IDE 配置目录
- 用户 home 根目录外部路径

---

## 21. 并发、取消与超时

## 21.1 并发模型

- `tokio::task` 管理任务
- 活跃 run 维护于并发安全状态表
- 每个 run 对应一个 child process / process group

## 21.2 取消

取消流程：

1. 标记状态为 `Cancelling`
2. 给 child process 发送中断/终止
3. 等待短暂 grace period
4. 超时后强制 kill
5. 写入取消原因与残余日志

## 21.3 超时

- `timeout_secs` 统一在 runtime 层生效
- provider 内部若也有超时，则以**更严格者**为准
- 超时后状态记为 `TimedOut`

---

## 22. 错误模型

## 22.1 错误分类

```rust
pub enum RuntimeErrorKind {
    SpecValidation,
    ProviderUnavailable,
    UnsupportedCapability,
    WorkspacePreparation,
    ContextCompilation,
    LaunchFailed,
    Timeout,
    Cancelled,
    SummaryParse,
    ArtifactRead,
    Internal,
}
```

## 22.2 错误返回原则

MCP 返回应包含：

- `kind`
- `message`
- `recoverable`
- `provider`
- `handle_id`（若已创建）
- `suggested_action`

### 例子

- provider binary 缺失 → recoverable=true（安装 CLI）
- experimental feature unavailable → recoverable=true（升级/启用 feature）
- summary parse failed → recoverable=true（保留原日志，允许人工读取）
- internal panic / invariant break → recoverable=false

---

## 23. rmcp 依赖策略

## 23.1 版本

MVP 基线锁定：

- `rmcp = 1.2.0`

## 23.2 MVP feature 集

建议从最小 feature 集开始：

```toml
rmcp = { version = "1.2.0", features = ["server", "macros", "transport-io"] }
```

### 说明

- `server`：服务端能力
- `macros`：`#[tool]` 等宏
- `transport-io`：stdio transport

HTTP transport 在最小 demo 验证后再加。

---

## 24. 运行时配置（runtime config）

建议支持可选全局配置：

- Linux/macOS：`~/.config/mcp-subagent/config.toml`
- Windows：对应 app config 目录

示例：

```toml
[server]
transport = "stdio"
log_level = "info"

[paths]
agents_dirs = ["./agents", "~/.config/mcp-subagent/agents"]
state_dir = "~/.local/share/mcp-subagent"

[providers.claude]
bin = "claude"
enabled = true

[providers.codex]
bin = "codex"
enabled = true

[providers.gemini]
bin = "gemini"
enabled = true
experimental_ok = true
```

---

## 25. 示例 agent 配置

```toml
[core]
name = "reviewer"
description = "Review code for correctness, regressions, and missing tests."
provider = "Codex"
model = "gpt-5.4"
instructions = """
Review like an owner.
Prioritize correctness, security, regressions, and missing test coverage.
Return concise findings with file references.
"""

allowed_tools = ["read", "grep", "glob"]
disallowed_tools = ["network"]
skills = ["reviewing", "security-basics"]

[runtime]
context_mode = "Isolated"
memory_sources = ["AutoProjectMemory"]
working_dir_policy = "GitWorktree"
file_conflict_policy = "Serialize"
sandbox = "ReadOnly"
approval = "ProviderDefault"
timeout_secs = 900
background_preference = "PreferForeground"

[provider_overrides.codex]
model_reasoning_effort = "high"
sandbox_mode = "read_only"
```

---

## 26. 开发顺序（推荐路线图）

## Phase 0：最小骨架

目标：

- `rmcp` stdio demo
- `list_agents`
- spec loader / validator
- provider probe
- mock runner

完成标准：

- 启动 `mcp-subagent --mcp`
- 能列出 agent
- 能运行一个 mock agent 并得到 `summary.json`

## Phase 1：Codex + Gemini

目标：

- `run_agent`
- `spawn_agent`
- `get_agent_status`
- `cancel_agent`
- `read_agent_artifact`
- `CodexRunner`
- `GeminiRunner`

完成标准：

- 支持真实本地 CLI 调用
- 结构化 summary 可解析
- 取消 / 超时 / 日志完整

## Phase 2：Claude

目标：

- `ClaudeRunner`
- 背景能力映射
- hooks / skills / inline MCP 的最小 bridge
- 更完善的 capability matrix

## Phase 3：写隔离与工程化增强

目标：

- `GitWorktree` 工作流
- 冲突检测
- patch artifact
- `doctor` / `validate` 完整化
- CI / e2e

## Phase 4：纯本地模型扩展

目标：

- `OllamaRunner`
- 纯本地离线能力
- provider-neutral memory 进一步增强

---

## 27. MVP 验收标准

MVP 必须满足：

1. `mcp-subagent --mcp` 可启动并被 MCP host 识别
2. 至少支持 `list_agents / run_agent / spawn_agent / get_agent_status / cancel_agent / read_agent_artifact`
3. 至少两个真实 provider runner 可运行（推荐先 Codex + Gemini）
4. 子 agent 不接收 raw parent transcript
5. `summary.json` 使用固定结构可被解析
6. 有 run 级日志、状态文件、artifact 索引
7. 取消和超时行为可测试
8. provider 不可用时有清晰错误
9. Gemini experimental 状态在 UI / API 中可见
10. 记忆去重机制至少覆盖“provider 原生文件不重复内联”

---

## 28. 风险与开放问题

## 28.1 已知风险

### R-001 CLI 语法变动

通过 probe、版本范围、集成测试缓解。

### R-002 provider 原生交互模式变化

通过 adapter 封装与 capability matrix 缓解。

### R-003 大仓库复制成本

通过 `git_worktree`、路径过滤、只读 agent in-place 缓解。

### R-004 写冲突

通过默认 `Serialize` 与 patch review 缓解。

### R-005 Gemini experimental 变化过快

通过 feature gate 与 provider probe 缓解。

### R-006 summary parse 不稳定

通过哨兵 JSON + degraded summary fallback 缓解。

## 28.2 当前不阻塞开发的开放问题

- 是否在 v0.x 引入 `plan_work / review_code / implement_change` 这类上层业务 tool
- 是否在 v0.x 暴露 `probe_providers` 为 MCP tool
- 是否需要 project-local state dir 模式
- 是否引入 SQLite 保存 run metadata（MVP 不必）

---

## 29. 最终建议（给开发同事）

对开发落地最重要的顺序是：

1. 先做 `rmcp` stdio 最小 server
2. 再做 spec/validation/probe
3. 然后把 ContextCompiler 与 Summary contract 做硬
4. 接着落地一个最稳定的真实 runner
5. 最后再碰 background、hooks、复杂权限和 worktree

### 不要一开始就做的事

- 不要先做 exporter
- 不要先做 HTTP transport
- 不要先做“全量共享记忆”
- 不要先做多 provider 字段大杂烩顶层 struct
- 不要先把 CLI 语法写死在业务逻辑里

---

## 30. 结论

`mcp-subagent` 的最终定位，不是“另一个 API 封装器”，而是：

> **一个以 MCP 为外壳、以本机 CLI 为执行通道、以 ContextCompiler 为核心、以多 provider runner 为扩展面的本地 agent runtime。**

它的价值来自四件事：

1. **把多 provider 的本地 CLI 使用方式统一起来**
2. **把上下文污染问题通过隔离与摘要回传解决掉**
3. **把并发、取消、日志、artifact 等工程细节收口到 runtime**
4. **为未来纯本地模型接入保留干净扩展点**

这份 v0.5 文档可以直接作为 `mcp-subagent-rs` 的开发基线。

---

## 附录 A：推荐的 Rust 依赖

```toml
[dependencies]
anyhow = "1"
thiserror = "2"
async-trait = "0.1"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["fmt", "env-filter"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
toml = "0.8"
uuid = { version = "1", features = ["v7", "serde"] }
time = { version = "0.3", features = ["serde", "formatting", "parsing"] }
directories = "5"
schemars = "0.8"
rmcp = { version = "1.2.0", features = ["server", "macros", "transport-io"] }
```

---

## 附录 B：StructuredSummary JSON 示例

```json
{
  "summary": "Reviewed the target module and found two correctness risks and one missing test area.",
  "key_findings": [
    "Null handling in src/parser.rs can panic when config is absent.",
    "Retry logic in src/client.rs ignores max_attempts under timeout pressure."
  ],
  "artifacts": [
    {
      "path": "artifacts/reviewer/report.md",
      "kind": "report_markdown",
      "description": "Detailed review report",
      "media_type": "text/markdown"
    }
  ],
  "open_questions": [
    "Should timeout behavior prefer fail-fast or silent retry?"
  ],
  "next_steps": [
    "Add regression tests for missing config and timeout retry."
  ],
  "exit_code": 0,
  "verification_status": "Partial",
  "touched_files": [
    "src/parser.rs",
    "src/client.rs",
    "tests/client_retry.rs"
  ]
}
```

---

## 附录 C：官方资料（开发前建议再次核对）

1. Claude Code Docs — Create custom subagents  
   <https://code.claude.com/docs/en/sub-agents>

2. Claude Code Docs — Authentication  
   <https://code.claude.com/docs/en/authentication>

3. Claude Code Docs — How Claude remembers your project  
   <https://code.claude.com/docs/en/memory>

4. OpenAI Codex Docs — Subagents  
   <https://developers.openai.com/codex/subagents>

5. OpenAI Codex Docs — Subagent concepts  
   <https://developers.openai.com/codex/concepts/subagents>

6. OpenAI Codex Docs — Configuration Reference  
   <https://developers.openai.com/codex/config-reference>

7. OpenAI Codex Docs — Custom instructions with AGENTS.md  
   <https://developers.openai.com/codex/guides/agents-md>

8. OpenAI Codex Docs — CLI  
   <https://developers.openai.com/codex/cli>

9. Gemini CLI Docs — Subagents (experimental)  
   <https://geminicli.com/docs/core/subagents/>

10. Gemini CLI Docs — Authentication  
    <https://geminicli.com/docs/get-started/authentication/>

11. Gemini CLI Docs — Provide context with GEMINI.md files  
    <https://geminicli.com/docs/cli/gemini-md/>

12. Docs.rs — rmcp 1.2.0  
    <https://docs.rs/crate/rmcp/latest>

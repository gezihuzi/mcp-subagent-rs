
# mcp-subagent 技术迭代文档 v0.10（2026.03）

**项目**：`mcp-subagent`  
**仓库**：`mcp-subagent-rs`  
**文档状态**：下一轮实现基线  
**目标定位**：从“能跑的多 LLM 子代理运行时”升级为“可观察、可解释、可中断、可直接上手的本地 Beta”

---

## 0. 本轮拍板结论

v0.9 之后，系统内核已经基本够用。下一轮不应继续优先扩 provider 或继续堆抽象，而应把重点转到：

1. **可观察性（observability）**
2. **简单任务的轻量化委派（minimal delegation）**
3. **原生输出保真（native-first results）**
4. **更顺手的任务生命周期命令面（submit/watch/result/stats）**
5. **provider 启动前置检查、阻塞原因显式化、使用统计可见化**

**一句话目标**：  
让一次子任务调用不再像黑盒，而像一个**有阶段、有心跳、有日志、有原生结果、有统计、有明确阻塞原因**的小型本地 job system。

---

## 1. 当前阶段问题复盘

基于当前行为反馈，本轮暴露的是**体验与运行时协议问题**，不是核心架构问题。

### 1.1 `spawn` 不应该“看起来卡住”

当前典型现象：

```bash
mcp-subagent spawn fast-researcher \
  --task "Search the official website of Octoclip and return JSON: {name,url,description}" \
  --json
```

用户看到：

- 只打印一行 `INFO starting command: spawn`
- 很久没有进一步输出
- 不知道 handle 是否已创建
- 不知道当前在做什么
- 不知道是不是卡住
- 不知道该取消还是继续等

这说明当前 `spawn` 的**接受（accepted）语义**没有和**执行（execution）语义**彻底分离。

### 1.2 异步任务缺少“活着”的证据

当前 `ps` / `status` 只能看到一个粗粒度的 running/succeeded/failed。  
对用户和主代理来说，这不够。缺的是：

- 当前阶段是什么
- 最后一次活动时间
- 最近有无 stdout/stderr
- provider 是否已真正启动
- 是否正在等待 trust / auth / consent / tool approval
- 是否已经开始产出 token
- 是慢，还是堵住了

### 1.3 Gemini 的“慢”很可能不是任务本身，而是启动环境前置阶段

根据实际现象，Gemini CLI 手动进入时先打印了：

- keychain fallback
- cached credentials
- 大量 skill conflict 警告
- 首次/当前目录 trust 相关行为
- 最后才进入任务交互

这说明“简单 research 任务”被额外绑上了：

- 工作区 trust 检查
- workspace / user / extension skills 发现与冲突处理
- 本地项目级配置扫描
- 可能的 workspace MCP / 扩展发现
- 人工看不到的 provider 启动噪音

对于一个**只需要查官网并返回 JSON** 的任务，这个前置成本明显偏高。

### 1.4 统一输出层仍需进一步改成 native-first

当前用户实际关心的是两件事：

- 子代理能否顺利完成任务
- 主管模型能否拿到可靠结果继续推进

如果 provider 已经给出了对主管足够有用的原生回答，但 runtime 因包装层解析失败把任务整体打成失败，这就是错误的优先级。

下一轮必须把结果模型固定为：

- **native_result**：永远保留，不丢
- **normalized_result**：尽力生成，可降级
- **run_status** 与 **normalization_status** 解耦

### 1.5 统计信息还不是一等对象

当前最需要补齐的是：

- wall time
- queue time
- startup/preflight time
- provider boot time
- first output latency
- parse time
- input/output tokens
- cache reads
- per-model usage breakdown
- tool / api / model active 时间

因为用户实际已经拿到了 Gemini CLI 自带的 wall time / active time / API time / tool time / per-model token breakdown，这说明至少部分 provider 已经能给出足够多的统计数据，应尽量收集而不是丢弃。

---

## 2. v0.10 总目标

### 2.1 核心目标

把 `mcp-subagent` 从“异步黑盒任务执行器”升级为：

> **可观察的本地多模型 Agent Job Runtime**

### 2.2 交付标准

一个任务从发起到完成，必须满足以下标准：

1. `spawn` 在 **300ms 内** 返回 `handle_id`（本地磁盘异常除外）
2. 任务在 **2 秒内** 至少产生一条阶段事件或 runtime heartbeat
3. 用户可以在任意时刻知道：
   - 当前阶段
   - 最近活动
   - 是否等待 provider / trust / auth / approval
   - 最近 stderr/stdout 摘要
4. provider 原生输出永久保留
5. 归一化失败不等于任务失败
6. `stats` 可直接看 wall/token/阶段耗时
7. 对于简单 research-only 任务，默认不加载多余 plan / skills / workspace discovery

---

## 3. 下一轮设计主线：从“运行任务”改成“运行可观察的 job”

---

## 4. 新的任务生命周期模型

### 4.1 RunState（顶层状态）

```rust
pub enum RunState {
    Accepted,
    Queued,
    Preparing,
    Launching,
    Running,
    Parsing,
    Succeeded,
    SucceededDegraded,
    Failed,
    Cancelled,
    TimedOut,
}
```

### 4.2 RunPhase（细粒度阶段）

```rust
pub enum RunPhase {
    Accepted,
    Queueing,
    WorkspacePrepare,
    ContextCompile,
    ProviderProbe,
    ProviderBoot,
    WaitingForConsent,
    WaitingForAuth,
    WaitingForTrust,
    WaitingForToolApproval,
    Running,
    ParsingOutput,
    PersistingArtifacts,
    Completed,
}
```

### 4.3 设计原则

- `state` 用于高层控制流
- `phase` 用于人类与主代理观察
- 一个 `state=Running` 的任务，phase 可以在：
  - `ProviderBoot`
  - `WaitingForTrust`
  - `WaitingForToolApproval`
  - `Running`
  之间切换

---

## 5. `spawn` 语义重构（P0）

### 5.1 新语义

`spawn` 的职责仅限于：

1. 校验 agent spec
2. 生成 handle_id
3. 创建 run 目录
4. 写入最小 run metadata
5. 将任务放入执行队列
6. **立即返回**

### 5.2 禁止在 `spawn` 同步路径中做的事

以下都必须挪到后台 worker：

- workspace 拷贝
- context compile
- provider probe
- CLI spawn
- stdout/stderr reader 启动等待
- summary parse
- artifact finalize

### 5.3 期望返回

```json
{
  "handle_id": "019d....",
  "state": "accepted",
  "phase": "Accepted",
  "queued_at": "2026-03-25T08:11:16.999Z"
}
```

### 5.4 测试要求

新增集成测试：

- mock runner 在后台 sleep 60s
- `spawn --json` 必须在 300ms 内返回
- handle 文件与 events.jsonl 必须已落盘
- 后台任务继续推进，不影响调用者 shell

---

## 6. 事件总线与可观察性（P0）

### 6.1 每个 run 必须有事件流

目录：

```text
state_dir/runs/<handle_id>/
  run.json
  events.jsonl
  stdout.txt
  stderr.txt
  native.jsonl         # 可选：provider 原生事件
  summary.json
  result.json
  stats.json
  artifacts.json
```

### 6.2 事件模型

```rust
pub struct RunEvent {
    pub seq: u64,
    pub ts: DateTime<Utc>,
    pub level: EventLevel,
    pub state: RunState,
    pub phase: RunPhase,
    pub source: EventSource,
    pub message: String,
    pub detail: Option<serde_json::Value>,
}
```

### 6.3 EventSource

```rust
pub enum EventSource {
    Runtime,
    Workspace,
    Context,
    Provider,
    Parser,
    Persistence,
}
```

### 6.4 必备事件类型

至少覆盖：

- run.accepted
- run.queued
- workspace.prepare.started
- workspace.prepare.completed
- context.compile.started
- context.compile.completed
- provider.probe.started
- provider.probe.completed
- provider.boot.started
- provider.first_output
- provider.waiting_for_trust
- provider.waiting_for_auth
- provider.waiting_for_tool_approval
- provider.stdout.delta
- provider.stderr.delta
- provider.heartbeat
- parse.started
- parse.completed
- run.completed
- run.failed
- run.cancelled
- run.timed_out

---

## 7. 心跳与黑盒感消除（P0）

### 7.1 Runtime heartbeat

即使 provider 完全没有输出，runtime 也要自己发心跳：

- `Preparing / Launching` 阶段：每 2 秒
- `Running` 阶段：每 5 秒
- 心跳内容至少包含：
  - 当前 phase
  - 已用 wall time
  - 距上次 provider 输出的时间
  - 当前 pid（如果已启动）
  - 最近一个已知动作摘要

示例：

```json
{
  "seq": 8,
  "ts": "...",
  "level": "Info",
  "state": "Launching",
  "phase": "ProviderBoot",
  "source": "Runtime",
  "message": "still alive",
  "detail": {
    "elapsed_ms": 8342,
    "pid": 91234,
    "last_provider_output_ms_ago": null
  }
}
```

### 7.2 First-byte watchdog

新增配置：

```rust
pub struct ObservabilityPolicy {
    pub first_output_warn_after_secs: u64,   // 默认 8
    pub first_output_fail_after_secs: u64,   // 默认 60（可 provider 覆盖）
    pub heartbeat_prelaunch_secs: u64,       // 默认 2
    pub heartbeat_running_secs: u64,         // 默认 5
}
```

若 provider 在 `first_output_warn_after_secs` 内没有任何 stdout/stderr/native event：

- 写入 warning 事件
- status 中显示 `stalled=true`
- 推荐可能原因：
  - auth required
  - trust prompt
  - skill discovery scan
  - workspace too heavy
  - provider startup failure

---

## 8. `watch` / `logs` / `events` / `wait`：让异步任务能看

### 8.1 新命令面（强烈建议）

保留兼容别名，但主命令面收敛为：

```bash
mcp-subagent spawn <agent> --task "..."
mcp-subagent watch <handle_id>
mcp-subagent wait <handle_id>
mcp-subagent result <handle_id>
mcp-subagent logs <handle_id> --follow
mcp-subagent events <handle_id> --follow
mcp-subagent stats <handle_id>
mcp-subagent cancel <handle_id>
mcp-subagent ps
```

### 8.2 `watch`

默认展示：

- state / phase
- elapsed
- last activity
- 最近几条 runtime/provider event
- 若已完成，自动显示 summary + result path + stats

### 8.3 `events --follow`

输出原始 JSONL，适合脚本与 MCP 调用者轮询。

### 8.4 `logs --follow`

按人类可读方式合并：

- runtime events
- stdout/stderr 摘要
- provider wait reasons
- parser decisions

### 8.5 `wait`

阻塞直到完成，退出码映射到任务结果。

---

## 9. 结果模型重构：native-first（P0）

### 9.1 新的结果对象

```rust
pub struct RunResult {
    pub handle_id: String,
    pub state: RunState,
    pub phase: RunPhase,
    pub exit_code: Option<i32>,
    pub native_result: NativeResult,
    pub normalized_result: Option<NormalizedResult>,
    pub normalization_status: NormalizationStatus,
    pub stats: RunStats,
    pub artifact_index: Vec<ArtifactEntry>,
}
```

### 9.2 NativeResult

```rust
pub struct NativeResult {
    pub provider: Provider,
    pub model: Option<String>,
    pub content_type: NativeContentType,
    pub content_text: Option<String>,
    pub content_json: Option<serde_json::Value>,
    pub raw_stdout_path: Option<PathBuf>,
    pub raw_stderr_path: Option<PathBuf>,
    pub raw_events_path: Option<PathBuf>,
}
```

### 9.3 NormalizationStatus

```rust
pub enum NormalizationStatus {
    NotRequested,
    Parsed,
    ParsedWithLoss,
    FailedButNativeAvailable,
    FailedNoNative,
}
```

### 9.4 拍板规则

- provider 成功、native_result 有内容、但 normalized parse 失败  
  => **state = SucceededDegraded**
- provider 成功、normalized_result 也成功  
  => **state = Succeeded**
- provider 失败，但 stderr/stdout 有可用信息  
  => **state = Failed**，但 native_result 仍保留
- 不允许“因为包装层失败而吞掉 provider 原生结果”

---

## 10. 统一输出支持策略（P0）

### 10.1 每个 provider 支持两种模式

```rust
pub enum OutputMode {
    NativeText,
    NativeJson,
    NativeStreamJson,
}
```

### 10.2 运行时策略

```rust
pub enum ParsePolicy {
    NativeOnly,
    BestEffortNormalize,
    RequireNormalized,
}
```

### 10.3 默认拍板

- `run` 默认：`BestEffortNormalize`
- `spawn` 默认：`BestEffortNormalize`
- `research-only` 简单任务：优先 `NativeJson` 或 `NativeText`
- 只有明确声明需要严格 schema 时，才用 `RequireNormalized`

---

## 11. Provider 适配策略：重点修 Gemini 黑盒启动

### 11.1 Gemini 的问题不是“模型慢”，而是“启动前置成本不透明”

下一版必须把以下行为变成显式 phase / event：

- trusted folder 检查
- keychain fallback
- cached credentials load
- skills discovery
- workspace/user/extension 冲突提示
- workspace setting / MCP / extension loading
- approval mode 阻塞
- first tool call 前等待

### 11.2 新增 ProviderBlockReason

```rust
pub enum ProviderBlockReason {
    TrustRequired,
    AuthRequired,
    ToolApprovalRequired,
    WorkspaceScan,
    SkillDiscovery,
    UnknownStartupWait,
}
```

一旦 stderr/stdout 命中可识别模式，立刻写事件：

```json
{
  "state": "Launching",
  "phase": "WaitingForTrust",
  "source": "Provider",
  "message": "gemini workspace trust is required before full functionality is available"
}
```

### 11.3 Gemini research-only 任务默认不应跑在项目根目录

#### 新策略：Scratch Workspace Mode

对于以下任务：

- `research-only`
- `web lookup`
- `single JSON lookup`
- `no repo read`
- `no file write`

默认运行在稳定 scratch 目录，而不是当前仓库：

```text
~/.mcp-subagent/provider-workspaces/gemini/research/
```

#### 原因

这样可以减少：

- 项目级 `.agents/skills` / `.gemini/skills` 扫描
- workspace trust 弹框
- 项目内 MCP / extension / custom commands 加载
- 共享 repo 技能冲突噪音
- 简单任务误吃项目上下文

### 11.4 稳定 scratch 目录必须“长期复用”，而不是每次临时目录

否则启用了 trusted folders 时，每个新目录都可能再次触发 trust 成本。

---

## 12. 默认委派策略：简单任务优先 minimal delegation（P0）

### 12.1 新增 DelegationProfile

```rust
pub enum DelegationProfile {
    Minimal,        // 默认：轻量子任务
    Standard,       // 普通开发任务
    WorkflowRich,   // 需要 plan/review/archive 的复杂任务
}
```

### 12.2 默认拍板

#### 子代理默认使用 `Minimal`

行为：

- 不继承 ActivePlan
- 不自动继承 skills
- 不自动启用 provider-native discovery
- 不自动加载 workspace memory
- 只吃主管给的 task + 可选 selected files + 可选 selected memory

### 12.3 只有满足以下条件才升级到 Standard / WorkflowRich

- 多文件修改
- 跨模块变更
- 需要 review
- 需要 plan refs
- 需要长期归档
- 需要 repo context
- 需要工具链执行

---

## 13. 新的上下文/记忆默认值（P0）

### 13.1 旧默认值问题

对于简单任务，默认 `ActivePlan`、workspace discovery、skills discovery 都太重。

### 13.2 新默认值

```rust
default context_mode = Isolated
default memory_sources = []
default delegation_profile = Minimal
default native_discovery = Disabled
default output_mode = NativeJson (if provider supports), else NativeText
default parse_policy = BestEffortNormalize
```

### 13.3 显式 opt-in 的项

以下必须显式打开：

- `ActivePlan`
- `ArchivedPlans`
- `skills`
- `provider native memory`
- `workspace discovery`
- `workflow-rich routing`

---

## 14. 工作流层不删，但不再强加给所有子代理

### 14.1 保留

- Research → Plan → Build → Review → Archive
- ActivePlan 机制
- stage-aware dispatcher

### 14.2 调整

它应当服务于：

- 主代理
- 复杂任务
- repo 级改动

而**不是每个简单子任务都默认带着 plan 和 workflow 跑**。

### 14.3 新增 WorkflowUsePolicy

```rust
pub enum WorkflowUsePolicy {
    Off,
    Auto,
    Required,
}
```

默认：

- `Minimal` => `Off`
- `Standard` => `Auto`
- `WorkflowRich` => `Required`

---

## 15. 统计模型（P0）

### 15.1 新的 RunStats

```rust
pub struct RunStats {
    pub queue_ms: u64,
    pub workspace_ms: u64,
    pub context_ms: u64,
    pub provider_probe_ms: u64,
    pub provider_boot_ms: u64,
    pub first_output_ms: Option<u64>,
    pub execution_ms: u64,
    pub parse_ms: u64,
    pub wall_ms: u64,

    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_tokens: Option<u64>,
    pub cache_write_tokens: Option<u64>,
    pub total_tokens: Option<u64>,

    pub api_ms: Option<u64>,
    pub tool_ms: Option<u64>,
    pub agent_active_ms: Option<u64>,

    pub provider_model_breakdown: Vec<ModelUsage>,
}
```

### 15.2 status / result / stats 都必须展示

#### `status`

轻量展示：

- state / phase
- elapsed
- first_output_ms
- last_event_at
- stalled?

#### `stats`

完整展示：

- 阶段耗时
- token
- provider breakdown
- raw usage source

---

## 16. 简单任务的自动降载策略（P1）

### 16.1 任务分类器

```rust
pub enum TaskClass {
    ResearchOnly,
    ReadOnlyRepo,
    WriteSmallPatch,
    MultiFileBuild,
    ReviewOnly,
    Unknown,
}
```

### 16.2 默认映射

#### `ResearchOnly`

- working_dir_policy = ScratchStable
- context_mode = Isolated
- memory_sources = []
- workflow_use_policy = Off
- native_discovery = Disabled
- parse_policy = BestEffortNormalize

#### `ReadOnlyRepo`

- working_dir_policy = InPlace
- context_mode = SelectedFiles / SummaryOnly
- native_discovery = Disabled by default

#### `WriteSmallPatch`

- working_dir_policy = GitWorktree
- workflow_use_policy = Auto

---

## 17. 命令面目标（P1）

### 17.1 最终 CLI 面向人类的默认体验

#### 同步执行（默认实时进度）

```bash
mcp-subagent run fast-researcher \
  --task "Search the official website of Octoclip and return JSON: {name,url,description}"
```

输出应类似：

```text
accepted  019d...
phase     ProviderBoot
agent     fast-researcher (gemini)
mode      minimal / research-only
workspace scratch: ~/.mcp-subagent/provider-workspaces/gemini/research

[08:11:17] workspace prepared
[08:11:17] context compiled
[08:11:18] launching gemini
[08:11:20] waiting for provider first output...
[08:11:25] provider startup slower than expected (possible trust/skill scan)
[08:11:29] provider reported first output
[08:11:42] tool: GoogleSearch
[08:11:58] parsing native result
[08:11:58] completed (succeeded)

RESULT
{"name":"Octoclip","url":"https://octoclip.com","description":"..."}
```

#### 异步执行

```bash
mcp-subagent spawn fast-researcher \
  --task "Search the official website of Octoclip and return JSON: {name,url,description}" \
  --json
```

立即返回：

```json
{
  "handle_id": "019d...",
  "state": "accepted",
  "phase": "Accepted"
}
```

然后：

```bash
mcp-subagent watch 019d...
mcp-subagent stats 019d...
mcp-subagent result 019d --json
```

### 17.2 `ps` 必须更有用

当前 running 时 `duration_ms=0` 不够好。  
下一版改成：

```text
019d... [running] phase=WaitingForTrust elapsed=18.4s last_event=2.1s ago stalled=yes provider=gemini agent=fast-researcher
task: Search the official website of Octoclip and return JSON: {name,url,description}
```

---

## 18. MCP 接口的下一轮增强（P1）

### 18.1 目前工具集保留

- list_agents
- run_agent
- spawn_agent
- get_agent_status
- cancel_agent
- read_agent_artifact

### 18.2 新增 / 重构建议

- `watch_agent_events(handle_id, since_seq?)`
- `get_agent_result(handle_id, mode=native|normalized|summary)`
- `get_agent_stats(handle_id)`
- `get_agent_logs(handle_id, stream=false, tail_lines=50)`

### 18.3 设计原则

MCP Host 不一定擅长长轮询复杂日志，所以工具必须：

- 支持增量读取
- 支持从 seq 继续
- 支持轻量摘要
- 支持区分 native / normalized 结果

---

## 19. 针对你的使用场景的默认预设（P1）

你当前典型场景：

- 主代理：Claude Code
- 主模型：`opusplan`
- 子代理：
  - Codex：后端 / correctness
  - Gemini：research / 前端 / 快速网页相关任务
  - Claude Sonnet：style / second opinion
- 重点目标：
  - 节省主管 token
  - 子代理返回可交差结果
  - 强记录 / 可复盘
  - 不能黑盒

### 19.1 推荐默认 agent team

- `supervisor-opusplan`
- `backend-coder`
- `fast-researcher`
- `frontend-builder`
- `correctness-reviewer`
- `style-reviewer`

### 19.2 拍板默认路由

- architecture / planning / tradeoff / spec => supervisor-opusplan
- backend implementation => backend-coder
- website lookup / docs lookup / API surface scan => fast-researcher
- frontend UI / interaction / quick visual impl => frontend-builder
- regression / edge case / correctness audit => correctness-reviewer
- style / readability / naming / consistency => style-reviewer

### 19.3 关键默认值

#### fast-researcher

- `delegation_profile = Minimal`
- `task_class = ResearchOnly`
- `working_dir_policy = ScratchStable`
- `native_discovery = Disabled`
- `workflow_use_policy = Off`
- `output_mode = NativeJson if supported`

#### backend-coder

- `delegation_profile = Standard`
- `working_dir_policy = GitWorktree`
- `workflow_use_policy = Auto`

---

## 20. 兼容 provider 原生能力的实现建议（P1）

### 20.1 Claude

- watched run 使用 `stream-json`
- quiet run 使用 `json`
- 最终可解析 `duration_ms`、`duration_api_ms`、`num_turns`、`session_id`

### 20.2 Gemini

- watched run 优先 `stream-json`
- quiet run 优先 `json`
- 当 stream/json 不稳定时，保留 text fallback，但不能丢 native result
- 捕捉 provider 自带 stats

### 20.3 Codex

- watched run 使用事件流 / `--json`
- quiet run 可用 final message 模式
- 保留 stderr activity 与最终 message 双轨

---

## 21. 需要新增的实现层模块（P1）

```text
src/runtime/
  event_bus.rs
  progress.rs
  stats.rs
  stall_detector.rs
  native_result.rs
  provider_blocks.rs
```

### 21.1 event_bus.rs

负责事件落盘与订阅。

### 21.2 progress.rs

将 runtime + provider 原始事件折叠成用户可读的 phase / live messages。

### 21.3 stats.rs

统一聚合 provider usage 和 runtime timing。

### 21.4 stall_detector.rs

检测“无输出但未退出”的运行时状态。

### 21.5 provider_blocks.rs

把 Gemini trust / approval、Claude permission wait、Codex approval 或 command wait 这些阻塞原因标准化。

---

## 22. 文档与 onboarding 改造（P1）

下一版必须把 README 重写成“最短成功路径”。

### 22.1 Happy Path（必须放最前）

#### 1. 初始化

```bash
mcp-subagent init --preset claude-opus-supervisor
```

#### 2. 先做健康检查

```bash
mcp-subagent doctor
```

#### 3. 运行一个简单 research 任务

```bash
mcp-subagent run fast-researcher \
  --task "Search the official website of Octoclip and return JSON: {name,url,description}"
```

#### 4. 异步版本

```bash
id=$(mcp-subagent spawn fast-researcher \
  --task "Search the official website of Octoclip and return JSON: {name,url,description}" \
  --json | jq -r '.handle_id')

mcp-subagent watch "$id"
```

### 22.2 必须单列一节：为什么 Gemini 可能启动更慢

清楚告诉用户：

- trusted folder
- workspace skills scan
- skill conflicts
- extension discovery
- provider startup noise

并给出建议：

- research 任务走 scratch workspace
- 预先 trust 稳定 scratch 目录
- 禁用不必要 workspace skills
- 用 `watch`/`events` 看是否堵在 trust 或 discovery

---

## 23. 测试计划（P0/P1）

### 23.1 P0 必测

1. `spawn` 立即返回测试
2. heartbeat 在 provider 无输出时仍持续产生
3. provider 首字节超时 warning
4. native result 保留测试
5. normalized 失败但 native 成功 => `SucceededDegraded`
6. `watch` 能看到 phase 演进
7. `stats` 能看到 wall_ms / queue_ms / provider_boot_ms
8. Gemini trust prompt 模式识别（fixture）
9. skill conflict stderr 模式识别（fixture）

### 23.2 P1 必测

1. ResearchOnly 默认跑 scratch stable workspace
2. Minimal delegation 不带 ActivePlan / skills
3. `ps` 显示 last_event_age / phase / stalled
4. `events --follow` 和 `logs --follow` 工作正常
5. MCP `get_agent_stats` / `watch_agent_events` 增量读取

---

## 24. 发布门槛（Release Gate）

v0.10 / 下一版可对外说“可直接使用”，至少要满足：

- `spawn` 不再黑盒卡住
- `watch` 可用
- `stats` 可用
- Gemini simple research 任务不会因为 workspace skills/trust 扫描变得不可解释
- native result 不会因为归一化失败而被判死刑
- README 有 5 分钟 happy path
- 至少一个 preset（你的 Claude supervisor 场景）可顺手使用

---

## 25. 实现顺序（拍板）

### Phase A（必须先做）

1. `spawn` accepted 语义重构
2. event bus + heartbeat + first-byte watchdog
3. native-first result model
4. `watch` / `events` / `stats`
5. Gemini block reason 识别
6. scratch stable workspace for research-only

### Phase B

1. status / ps 体验升级
2. minimal delegation 成为默认
3. workflow rich 改为 opt-in
4. provider usage 聚合

### Phase C

1. MCP 增量事件读取
2. README / onboarding 重写
3. 更好的 preset 与脚本

---

## 26. 最终拍板

下一轮不要再优先做“更复杂的多 Agent 编排”。  
先把下面三件事做扎实：

1. **任务立刻可见**
2. **运行过程可观察**
3. **结果与统计可解释**

只要这三件事做对，`mcp-subagent` 就会从“架构上对”变成“每天真愿意用”。

对于你现在的实际工作流，我的明确拍板是：

- 主代理继续用 Claude Code `opusplan`
- research 子代理默认走 **Gemini + scratch stable workspace + minimal delegation**
- backend/correctness 子代理默认走 Codex
- 复杂 repo 任务才启用 workflow-rich
- 所有子任务都必须能 `watch`、能 `stats`、能看到 block reason、能拿到 native result

这会比继续堆更多 provider 或更多抽象，实际收益更大。

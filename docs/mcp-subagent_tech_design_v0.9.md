# mcp-subagent（mcp-subagent-rs）技术设计文档 v0.9

**状态**：下一版目标文档（基于 develop 分支审阅后拍板）  
**目标定位**：首个“直接可用”的**本地多 LLM 委派运行时 Beta**  
**仓库名**：`mcp-subagent-rs`  
**crate / 二进制名**：`mcp-subagent`

---

## 0. 这版为什么要改：先拍板

v0.8 之后，内核架构已经基本成型，但现在暴露出两个真正影响可用性的核心问题：

1. **委派上下文过重**  
   当前默认 `memory_sources = [AutoProjectMemory, ActivePlan]`，会让子代理天然继承 `PLAN.md`；同时 provider 原生的 skills / memory / workspace 发现机制也会把大量环境信息带进去。  
   这对“主管派发单任务，子代理轻量执行”的场景不理想。

2. **输出包装过重**  
   当前 prompt/summary 管道默认要求 sentinel 包裹 JSON，解析失败就降级为 Invalid。  
   实际上 provider 往往已经给出了正确的“人可读”或“可供主管消费”的结果，但包装层没有吃到，于是任务被判失败。

**v0.9 的核心拍板**：

- **默认委派策略改成：轻委派（delegation-minimal）**
- **默认输出策略改成：原生优先（native-first），归一化其次（normalize-second）**
- **默认可观测性补齐：耗时、退出码、重试、token/usage（能拿真值就拿真值，拿不到就诚实标未知或估算）**
- **命令体验收口：让开发者不需要先理解内部状态机，先把任务跑起来**

---

## 1. v0.9 目标

### 1.1 产品目标

让 `mcp-subagent` 达到下面这个“第一次成功路径”：

- 开发者用 `init --preset` 生成一套可用团队
- 用 `connect --host claude|codex|gemini` 一键输出接入命令
- 在 Claude Code / Codex CLI / Gemini CLI 中把 `mcp-subagent` 注册为本地 stdio MCP 服务
- 主模型通过 MCP 发起子任务
- 子任务默认以**最轻上下文**运行
- 子任务结果默认以**原生结果 + 归一化摘要**同时落盘
- 开发者可以简单地查看：
  - 状态
  - 结果
  - stderr/stdout
  - 耗时
  - token/usage（如果可得）
- 如果解析失败，也**不应该因为包装层而把一个本来有价值的结果判成硬失败**

### 1.2 主要适配场景

重点服务这一类工作流：

- **主管模型**：Claude Code（优先 `opusplan`，更重的架构评估时可手动切到 `opus`）
- **子代理**
  - `backend-coder`：Codex
  - `correctness-reviewer`：Codex
  - `frontend-builder`：Gemini `pro`
  - `fast-researcher`：Gemini `flash`
  - `style-reviewer`：Claude `sonnet`
  - `local-fallback-coder`：Ollama

这是一个“主管高智商 + 执行代理更省 + reviewer 纠偏”的典型结构。

---

## 2. 对当前 develop 的最终判断

## 2.1 已经做对的部分

当前 develop 分支已经具备这些重要能力：

- 分层 spec（core / runtime_policy / provider_overrides）
- stdio MCP server
- workflow / plan gate
- memory resolver
- workspace policy（in-place / worktree / temp_copy / auto）
- 真实 provider runners（Codex / Claude / Gemini / Ollama）
- doctor / validate / init / connect-snippet / connect
- 状态持久化与 artifact 落盘

这意味着项目已经从“架构试验”进入“产品收口”阶段。

## 2.2 当前真正的问题，不是架构，而是默认行为

当前版本最主要的问题不是“不会跑”，而是：

- **默认上下文偏重**
- **默认输出契约偏严格**
- **状态/结果查看体验偏底层**
- **对 provider 的原生 ambient context（skills / memory / extension）缺少隔离策略**

---

## 3. v0.9 的核心设计调整

## 3.1 默认委派策略：从 workflow-rich 改为 delegation-minimal

### 新原则

**子代理默认只做主管明确交代的单任务。**

### 必须明确区分的两个层面

1. **任务工件层（workflow / plan）**
2. **委派执行层（subagent execution）**

当前系统的问题，是把“workflow 很重要”直接等价成“每个子代理都应该默认看到 ActivePlan”。

这是不对的。

### v0.9 拍板

`ActivePlan` 不再作为所有子代理的默认 memory source。

#### 新默认

```toml
memory_sources = ["AutoProjectMemory"]
delegation_context = "minimal"
```

#### 解释

- `PLAN.md` 仍然是 workflow 的公共控制面
- 但**是否把计划同步给子代理**，由主管明确决定
- 子代理不再默认继承完整计划
- 只有在以下场景下才显式注入 plan：
  - 需要执行 plan 的某一个 section
  - 需要 reviewer 对照 acceptance criteria 审查
  - 需要 archiver 归档总结
  - 主管显式要求 plan-aware 执行

### 新增：DelegationContextPolicy

```rust
pub enum DelegationContextPolicy {
    Minimal,         // 默认：只有 task brief + project memory
    SummaryOnly,     // 仅 brief + parent summary
    SelectedFiles,   // 仅 brief + selected files
    PlanSection,     // brief + plan 某 section + 相关 acceptance
    FullPlan,        // 明确允许时才用
    ProviderNativeOnly, // 尽量只依赖 provider 自带 project memory / cwd
}
```

### 行为约束

- `Minimal` 是默认值
- `FullPlan` 必须显式启用
- `PlanSection` 必须引用具体 section id / title
- 委派路径上**禁止默认继承 raw transcript**
- 委派路径上**禁止默认全量同步 workflow 工件**

---

## 3.2 Skills 策略：从“能发现就让它发现”改为“默认不放大 ambient context”

当前实际问题不是 runtime 主动注入了很多 skills，而是 provider 自己会发现 skills。

尤其 Gemini CLI 当前有三层 skill 发现：

- workspace
- user
- extension

并且同名 skill 还有优先级覆盖逻辑。  
这对主会话是好事，但对“轻量单任务子代理”往往是坏事。

### v0.9 拍板

新增 `NativeDiscoveryPolicy`：

```rust
pub enum NativeDiscoveryPolicy {
    Inherit,     // 继承 provider 的默认发现机制
    Minimal,     // 尽量关闭扩展/skills/额外 ambient context
    Isolated,    // 尽量隔离 HOME/XDG/config/workspace 发现路径
    Allowlist,   // 只允许指定 memory/skills/extensions
}
```

### 新默认

对子代理默认使用：

```toml
native_discovery = "minimal"
```

### provider-specific 规则

#### Gemini

默认子代理执行时：

- 优先禁用 extensions
- 不额外开放 workspace skills
- 不自动把用户全局 skills 暴露给任务
- 需要 skills 时由 supervisor 显式要求

如果 Gemini CLI 当前版本支持 `--extensions none` / `-e none`，默认启用。  
如果仍然会自动发现 workspace/user skill，则通过隔离 HOME / XDG / config 路径进一步压缩。

#### Claude

Claude subagent 的 `skills` 字段如果显式配置，会把 full skill content 注入上下文。  
因此 runtime 不做“隐式 skill preload”。

#### Codex

Codex 的 agent/config/skills 也应该视为显式能力，而不是默认 ambient 注入。

### 工程结论

**skills 必须从“默认发现”变成“显式授权”。**

---

## 3.3 输出策略：从 strict-wrapper 改为 native-first

### 当前问题

现在的 prompt/summary pipeline 强调：

- 输出 sentinel
- JSON 契约
- 归一化 envelope

这在理想情况下很好，但真实运行里经常出现：

- 模型答对了
- 数据有价值
- 只是包装层没吃到
- 然后任务被判 Invalid / failed

### v0.9 拍板

默认输出模式改为：

```rust
pub enum OutputMode {
    NativeOnly,
    NormalizedOnly,
    Both,          // 默认
}

pub enum ParsePolicy {
    BestEffort,    // 默认
    Strict,
}
```

#### 默认值

```toml
output_mode = "both"
parse_policy = "best_effort"
```

### 新语义

- provider 原始 stdout/stderr 永远保留
- 归一化解析尽最大努力做
- **如果 provider exit_code == 0 且有可用原始结果，就不因为归一化失败而把任务整体判死**
- parse failure 只是：
  - `normalization_status = degraded`
  - `normalized_result = null`
  - `native_result` 仍然可用
- 只有以下情况才视为硬失败：
  - provider 进程失败
  - timeout / cancel
  - 明确开启了 `parse_policy = "strict"` 且归一化失败

### 新结果模型

```rust
pub struct RunResultEnvelope {
    pub handle_id: String,
    pub run_state: RunState,
    pub provider_exit_code: Option<i32>,
    pub normalization_status: NormalizationStatus,
    pub native_result: NativeResult,
    pub normalized_result: Option<StructuredResult>,
    pub usage: UsageStats,
    pub artifacts: Vec<ArtifactRef>,
}
```

### NormalizationStatus

```rust
pub enum NormalizationStatus {
    NotRequested,
    Valid,
    Degraded,
    Failed,
}
```

### provider 归一化策略

#### Codex

优先使用：

- `--output-schema`
- `--output-last-message`

如果 schema 不满足，但 final message 存在：

- 保留 final message
- 记为 `Degraded`

#### Claude

优先用 schema / structured contract  
如果当前 CLI 版本或运行模式下 schema 不稳定：

- 自动 fallback 到 raw text + best-effort parse

#### Gemini

优先尝试 provider 自带 JSON 输出能力  
如果 `--output-format json` / `stream-json` 可稳用，则优先依赖它  
否则走 raw text + best-effort parse

### 原则

**Runtime 的职责是“运输和整合结果”，不是“因为格式不理想而摧毁结果”。**

---

## 3.4 统计与可观测性：必须补 usage

当前要补的不是漂亮 dashboard，而是最基础的一致观测。

### 新增 `UsageStats`

```rust
pub struct UsageStats {
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<u64>,
    pub provider: String,
    pub model: Option<String>,
    pub provider_exit_code: Option<i32>,
    pub retries: u32,

    pub token_source: TokenSource,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub total_tokens: Option<u64>,

    pub estimated_prompt_bytes: Option<u64>,
    pub estimated_output_bytes: Option<u64>,
}
```

```rust
pub enum TokenSource {
    ProviderReported,
    Estimated,
    Unknown,
}
```

### 原则

- 有 provider 真值时，直接记录
- 没有真值时，给估算
- 估算要明确标识为 `Estimated`
- 完全拿不到就标 `Unknown`
- 不允许“装作知道 token”

### 命令面必须能直接看到

- `show <id>`
- `result <id> --json`
- `ps`
- MCP `get_run_status`
- MCP `get_run_result`

---

## 3.5 命令体验：从“底层工具集”变成“顺手”

当前命令不是不能用，而是偏内核。

### v0.9 新命令建议

#### 保留

- `run`
- `cancel`
- `artifact`
- `doctor`
- `validate`
- `init`
- `connect`
- `connect-snippet`
- `mcp`

#### 新增/重命名

- `submit`：异步提交（比 `spawn` 更直观）
- `ps`：列出 run
- `show <id>`：状态 + 摘要 + usage
- `result <id>`：看结果
- `logs <id>`：看 stdout/stderr
- `watch <id>`：等待直到结束
- `rm <id>`：删除某次运行记录
- `prune`：清理历史运行

### 旧命令兼容

- `spawn` 继续保留，但文档层面推荐 `submit`
- `status` 继续保留，但推荐 `show`

---

## 4. MCP 工具面 v0.9

当前 MCP tools 已够用，但对真实使用不够顺手。

### 新工具集

#### 保留

- `list_agents`
- `run_agent`
- `spawn_agent`
- `get_agent_status`
- `cancel_agent`
- `read_agent_artifact`

#### 新增

- `list_runs`
- `get_run_result`
- `read_run_logs`
- `watch_run`
- `get_agent_preset`
- `explain_agent_defaults`

### 设计原则

主代理不应该为了拿一个“最终结果”去自己拼装 `status + artifact`。

---

## 5. 新默认 preset：按你的场景拍板

## 5.1 推荐主管 preset

### `claude-opus-supervisor-minimal`

默认主管：

- Host：Claude Code
- model：`opusplan`
- role：架构 / 拆解 / 路由 / 汇总 / 审批

默认行为：

- 主模型维护 `PLAN.md`
- 只给子代理发最小化 brief
- reviewer 默认双审
- 非必要不把 full plan 注入 worker

## 5.2 推荐 agent 角色

### `fast-researcher`

```toml
provider = "gemini"
model = "flash"
delegation_context = "minimal"
native_discovery = "isolated"
output_mode = "both"
parse_policy = "best_effort"
sandbox = "read_only"
```

### `frontend-builder`

```toml
provider = "gemini"
model = "pro"
delegation_context = "selected_files"
native_discovery = "isolated"
output_mode = "both"
parse_policy = "best_effort"
sandbox = "workspace_write"
```

### `backend-coder`

```toml
provider = "codex"
model = "gpt-5.3-codex"
delegation_context = "selected_files"
output_mode = "both"
parse_policy = "best_effort"
sandbox = "workspace_write"
```

### `correctness-reviewer`

```toml
provider = "codex"
model = "gpt-5.3-codex"
delegation_context = "plan_section"
output_mode = "both"
parse_policy = "best_effort"
sandbox = "read_only"
```

### `style-reviewer`

```toml
provider = "claude"
model = "sonnet"
delegation_context = "selected_files"
output_mode = "both"
parse_policy = "best_effort"
sandbox = "read_only"
```

### `local-fallback-coder`

```toml
provider = "ollama"
model = "qwen2.5-coder"
delegation_context = "selected_files"
output_mode = "both"
parse_policy = "best_effort"
sandbox = "workspace_write"
```

---

## 6. ActivePlan 的新定位

`PLAN.md` 很重要，但不再是“默认全员可见”。

### 新规则

- `PLAN.md` 是主线程控制面
- 子代理只看：
  - brief
  - 相关 files
  - 必要时某个 plan section
- reviewer / archiver 可以看 plan
- worker 默认不看 full plan

### 新 memory source 规则

```rust
pub enum MemorySource {
    AutoProjectMemory,
    ActivePlan,
    ArchivedPlans,
    File(String),
    Glob(String),
    Inline(String),
}
```

**但默认不再包含 `ActivePlan`。**

---

## 7. Structured Output 的新 contract

v0.9 不再要求所有 provider 都严格吐同一份 JSON 才算成功。

### 统一 envelope

```rust
pub struct StructuredResult {
    pub summary: Option<String>,
    pub key_findings: Vec<String>,
    pub touched_files: Vec<String>,
    pub artifacts: Vec<String>,
    pub verification_status: Option<String>,
    pub open_questions: Vec<String>,
    pub next_steps: Vec<String>,
}
```

### 原则

- 这个结果是“方便主管模型消费的归一化视图”
- 它不是任务是否成功的唯一判据
- `native_result` 与 `normalized_result` 并存

---

## 8. CLI 使用示例（尽量最短）

## 8.1 初始化一个适合你场景的团队

```bash
mcp-subagent init --preset claude-opus-supervisor-minimal
```

生成：

- `agents/`
- `.mcp-subagent/`
- `PLAN.md` 模板
- README 接入说明
- 建议 `.gitignore`

## 8.2 查看连接命令

```bash
mcp-subagent connect-snippet --host claude
mcp-subagent connect-snippet --host codex
mcp-subagent connect-snippet --host gemini
```

## 8.3 直接连接到 Claude Code

```bash
mcp-subagent connect --host claude
```

## 8.4 最短同步运行

```bash
mcp-subagent run backend-coder \
  --task "Implement a POST /v1/todos endpoint in src/api/todos.rs and add tests." \
  --working-dir .
```

## 8.5 最短异步运行

```bash
mcp-subagent submit fast-researcher \
  --task "Search the official docs for Octoclip and return the product home URL and a one-paragraph description." \
  --working-dir /tmp/mcp-subagent-isolated
```

## 8.6 查看运行列表

```bash
mcp-subagent ps
```

## 8.7 查看一个运行

```bash
mcp-subagent show <handle_id>
```

输出应包含：

- 状态
- provider/model
- duration
- usage
- normalization_status
- summary（如果有）
- 关键 artifact 路径

## 8.8 看原始结果

```bash
mcp-subagent result <handle_id> --raw
mcp-subagent result <handle_id> --normalized
mcp-subagent result <handle_id> --summary
```

## 8.9 跟踪直到结束

```bash
mcp-subagent watch <handle_id>
```

## 8.10 看 stderr/stdout

```bash
mcp-subagent logs <handle_id> --stderr
mcp-subagent logs <handle_id> --stdout
```

---

## 9. Claude / Codex / Gemini 接入示例

## 9.1 Claude Code

```bash
claude mcp add --transport stdio mcp-subagent -- \
  /absolute/path/to/mcp-subagent \
  mcp
```

## 9.2 Codex CLI

```bash
codex mcp add mcp-subagent -- \
  /absolute/path/to/mcp-subagent \
  mcp
```

## 9.3 Gemini CLI

```bash
gemini mcp add mcp-subagent \
  /absolute/path/to/mcp-subagent \
  mcp
```

---

## 10. 主管如何调度子代理（推荐提示范式）

在 Claude Code 主会话里，你的默认工作方式建议是：

### 10.1 先规划

```text
先更新 PLAN.md，明确：
- Goal
- Scope
- Constraints
- Acceptance criteria
- Steps
- Review checklist
然后只把当前 step 所需的最小任务分派给子代理。
```

### 10.2 发给子代理的标准委派

```text
调用 mcp-subagent 的 <agent_name>。
只给它完成当前单任务所需的最小上下文，不要同步完整 plan，除非它必须对照某个 plan section 执行或审查。
返回时优先给我：
1. 结论
2. 修改文件
3. 风险/未完成项
4. 必要的原始输出或证据
```

### 10.3 reviewer 的标准委派

```text
对照 PLAN.md 的 acceptance criteria 审查当前改动。
不要重做实现。
重点检查：
- correctness
- regression risk
- missing tests
- mismatch with plan
```

---

## 11. v0.9 的实现要求（P0 / P1 / P2）

## P0：必须做完

1. `memory_sources` 默认移除 `ActivePlan`
2. 新增 `delegation_context`
3. 新增 `native_discovery`
4. 新增 `output_mode`
5. 新增 `parse_policy`
6. provider 成功 + parse 失败时不自动判 hard failure
7. 增加 `usage` / `duration_ms` / `provider_exit_code`
8. 新增 `submit / ps / show / result / logs / watch`
9. Gemini runner 增加“最小 ambient discovery”模式
10. README / connect-snippet / init 模板统一到新命令面

## P1：建议完成

1. MCP 新增 `list_runs / get_run_result / read_run_logs / watch_run`
2. `PlanSection` 支持 section selector
3. reviewer agent 默认附 acceptance criteria
4. `show` 支持彩色简洁输出
5. `result --json` 输出固定 schema

## P2：可后续补

1. run timeline / event stream
2. provider usage 更精确采集
3. richer retry policy
4. profile/preset marketplace
5. per-provider ambient isolation diagnostics

---

## 12. 验收标准

v0.9 完成的标准不是“代码看起来更高级”，而是：

### 12.1 实际使用验收

- 用 `init --preset claude-opus-supervisor-minimal` 初始化项目
- 用 Claude Code 注册 MCP 成功
- 主模型能发起至少 3 个子任务：
  - Gemini research
  - Codex backend coding
  - Claude style review
- 至少有 1 个任务在归一化失败时，仍能通过 raw result 被主管成功消费
- `show` 能直接看 duration / usage / normalization_status
- `logs` 能快速看到 provider stderr
- Gemini skill 冲突场景下，`native_discovery=minimal|isolated` 可以明显减少环境噪声

### 12.2 工程验收

- 单元测试覆盖：
  - delegation_context
  - parse_policy
  - native_discovery
  - result envelope
- 至少一个集成测试覆盖：
  - provider success + normalization degraded
- 文档示例必须能跑通

---

## 13. 对当前代码的具体迁移建议

### 13.1 runtime_policy

- 保留 `ContextMode`
- 新增 `DelegationContextPolicy`
- `default_memory_sources()` 改为只返回 `AutoProjectMemory`

### 13.2 context compiler

- prompt contract 从“强制 JSON 成功”改成“优先结构化，但允许 raw fallback”
- 添加 `parse_policy`

### 13.3 summary / normalization

- `invalid_envelope()` 不再直接隐含“run failed”
- 新增 `NormalizationStatus::Degraded`

### 13.4 provider runners

- Gemini：增加 ambient discovery 最小化参数/环境
- Codex：继续优先 `--output-schema` + `--output-last-message`
- Claude：schema 失败时更平滑 fallback

### 13.5 CLI / MCP

- 面向使用者的 happy path 重写
- 让“拿结果”变成一条命令，而不是 `status + artifact` 手拼

---

## 14. 最终拍板结论

v0.9 不再追求“把所有 workflow 智慧都默认塞给每个子代理”。  
**v0.9 的核心哲学是：**

> 主管重，子代理轻；  
> 默认最小委派，必要时再放大；  
> 原生结果优先保真，归一化只做增强，不做破坏；  
> 运行可观测，命令够顺手，才算真的可用。

这就是下一版应该直接实现的方向。

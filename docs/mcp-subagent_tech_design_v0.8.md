# mcp-subagent（mcp-subagent-rs）技术目标文档 v0.8

**文档定位**：下一迭代的实现目标文档。  
**目标版本**：v0.8（首个“可直接使用”的本地多 LLM Agent Runtime Beta）。  
**crate 名称**：`mcp-subagent`  
**仓库名**：`mcp-subagent-rs`

---

## 0. 当前状态结论（基于 v0.7 实现的评审）

### 0.1 总体判断

v0.7 已经不是“设计原型”，而是一个**可运行、可继续演进的本地 Agent Runtime 雏形**。

它已经具备：

- 单一 Rust 二进制 + 本地 stdio MCP server
- 统一 agent spec 分层（core / runtime / provider overrides / workflow）
- provider probe / doctor / validate / init / run / spawn / status / cancel / artifact 的命令面
- 上下文编译器、memory source、workflow gate、runtime depth、workspace policy、summary contract
- Claude / Codex / Gemini / Ollama / Mock 五类 runner 路径
- preset 初始化能力与多 Agent 模板

### 0.2 v0.7 的真实成熟度判断

**结论**：

- **架构成熟度**：高
- **可日常试用程度**：中高
- **对团队成员“开箱即用”程度**：中
- **距离“我装上就能直接用”的差距**：主要不在内核，而在**产品化收口、安装接入、示例、文档、默认预设、第一次成功路径**

所以 v0.8 的方向不应再以“继续补抽象”为主，而应以：

> **把现有能力压缩成一个最清晰、最可靠、最容易上手的默认体验。**

---

## 1. v0.8 产品目标

### 1.1 核心目标

v0.8 的目标不是“继续扩功能”，而是交付首个满足以下条件的版本：

> 一个开发者在本地安装好 Claude Code / Codex CLI / Gemini CLI 中任意一个主客户端后，能够在 10 分钟内把 `mcp-subagent` 接入进去，初始化一个可用团队预设，并成功让主 Agent 调用跨 provider 子代理完成真实任务。

### 1.2 v0.8 的一句话定义

**首个直接可用的本地多 LLM Agent Workflow Runtime Beta。**

### 1.3 非目标

v0.8 不做这些事情：

- 不追求 Claude / Codex / Gemini / Ollama 能力完全对齐
- 不实现 HTTP transport（继续坚持 stdio-first）
- 不引入复杂插件系统
- 不做远程调度平台
- 不做云端状态同步
- 不做“零风险 / 零幻觉”承诺

---

## 2. v0.8 拍板后的产品定位

### 2.1 两种使用模式

#### 模式 A：Host-Supervisor 模式（优先主路径）

外部主客户端作为“主管”：

- Claude Code
- Codex CLI
- Gemini CLI

`mcp-subagent` 作为 MCP server 挂进去，向主客户端暴露：

- `list_agents`
- `run_agent`
- `spawn_agent`
- `get_agent_status`
- `cancel_agent`
- `read_agent_artifact`

主客户端负责：

- 高层对话
- 需求理解
- 决策
- 任务分配
- 结果吸收

`mcp-subagent` 负责：

- 本地多 provider 子代理管理
- workflow 约束
- 上下文编译
- 工作目录隔离
- 产物归档
- 结构化 summary

#### 模式 B：Standalone Runtime 模式

不用任何主 MCP host，直接用命令行：

- `mcp-subagent init`
- `mcp-subagent validate`
- `mcp-subagent doctor`
- `mcp-subagent run`
- `mcp-subagent spawn`
- `mcp-subagent status`
- `mcp-subagent cancel`
- `mcp-subagent artifact`

这是调试、回归测试、CI、本地 smoke 和无 host 场景的兜底模式。

### 2.2 v0.8 的优先体验

v0.8 只重点优化一条默认 happy path：

> **Claude Code 作为主管（推荐用 `opusplan`），通过 MCP 调用 `mcp-subagent`，再由 `mcp-subagent` 调度 Codex / Gemini / Claude / Ollama 子代理。**

这是本版本的主打路径。

Codex / Gemini 作为主客户端也要支持，但不是 v0.8 唯一“最优文档路径”。

---

## 3. v0.8 的设计原则

### 3.1 原则一：默认隔离，不透传 raw transcript

保持 v0.7 的硬约束，不得回退：

- 子代理禁止接收父对话原始 transcript
- 只能注入编译后的 briefing / summary / selected files / memory sources
- plan 是共享工件，不是共享聊天记录

### 3.2 原则二：workflow 优先于 prompt

多 Agent 协作的锚点不是 prompt，而是工件：

- `PLAN.md`
- artifacts
- structured summary
- archived plans
- decision note / project memory

### 3.3 原则三：一个最强默认团队，优于十个半成品模板

v0.8 只重点做强以下默认团队：

- `claude-opus-supervisor`

其他 preset 保留，但优先级低于主 preset 的可用性与文档质量。

### 3.4 原则四：直接可用比“继续抽象”更重要

如果某个能力已经在架构上成立，但首次使用还要读半天代码才能接上，那 v0.8 视为未完成。

### 3.5 原则五：显式承认 provider 成熟度不一致

v0.8 不做“多 provider 完全平权”的假设。

本版本默认排序：

1. **Codex**：最适合做首个主力 build / correctness runner
2. **Claude**：适合主会话与 style / supervisory review
3. **Gemini**：适合 fast research / frontend builder，但仍按 experimental 心智对待
4. **Ollama**：本地 fallback / 可选增强，不当作默认主路径

---

## 4. v0.8 交付定义（Definition of Done）

v0.8 必须同时满足以下条件，才视为完成：

### 4.1 可安装

至少提供两种安装方式：

- `cargo install --git ...` 的开发安装方式
- GitHub Release 预编译二进制（macOS / Linux / WSL 至少覆盖）

### 4.2 可初始化

用户在空仓库执行：

```bash
mcp-subagent init --preset claude-opus-supervisor
```

能生成：

- `agents/`
- `PLAN.md`
- `.mcp-subagent/config.toml`
- `README.mcp-subagent.md`

### 4.3 可验证

下面命令必须可跑通：

```bash
mcp-subagent validate --agents-dir ./agents
mcp-subagent doctor --agents-dir ./agents
mcp-subagent list-agents --agents-dir ./agents
```

### 4.4 可接入

文档必须给出**可直接复制**的接入命令：

- Claude Code
- Codex CLI
- Gemini CLI

### 4.5 可执行

至少有一条端到端示例可以跑通：

- 主会话（推荐 Claude Code）
- 调 `spawn_agent` / `run_agent`
- 子代理完成 Build 或 Review
- 结果写入 state/artifacts
- 可读 summary / log / report

### 4.6 可复盘

完成一次 workflow 后，至少能看到：

- active `PLAN.md`
- 结构化 summary
- final summary / archived plan（如果 stage 包含 Archive）
- artifacts 索引

---

## 5. v0.8 必须解决的问题（按优先级）

---

### 5.1 P0：第一次成功路径必须变得非常简单

#### 当前问题

虽然 v0.7 已经有 `init` 和 `README.mcp-subagent.md` 生成，但对新用户来说，第一次把本地二进制挂到 Claude / Codex / Gemini 仍然不够“无脑”。

尤其是：

- 接入命令需要用户自己拼可执行路径
- 绝对路径 / 项目路径 / state_dir 路径需要自己理解
- README 需要保证命令模板可以直接复制使用
- 需要一个“安装后下一步做什么”的明确顺序

#### v0.8 要求

新增至少一个下面两者之一：

##### 方案 A：`doctor --print-connect-snippets`

```bash
mcp-subagent doctor --agents-dir ./agents --print-connect-snippets
```

输出：

- Claude Code 接入命令
- Codex CLI 接入命令
- Gemini CLI 接入命令
- 使用的绝对路径
- 项目 agents / state 路径

##### 方案 B：`mcp-subagent connect-snippet`

```bash
mcp-subagent connect-snippet --host claude
mcp-subagent connect-snippet --host codex
mcp-subagent connect-snippet --host gemini
```

**拍板建议**：优先做 **方案 B**，因为更直观。

#### 强约束

生成的 snippet 必须：

- 使用当前机器上的二进制绝对路径
- 使用当前工作区绝对路径
- 不留 `/agents`、`/.mcp-subagent/state` 这种示意占位路径
- 可直接复制运行

---

### 5.2 P0：README 模板必须成为真正可执行的 onboarding 文档

`init` 生成的 `README.mcp-subagent.md` 在 v0.8 必须升级成“第一次成功指南”，而不只是说明书。

#### 必须包含

1. 当前 preset 的角色说明
2. 三步 quick start
3. 本地 standalone smoke
4. 连接 Claude 的命令
5. 连接 Codex 的命令
6. 连接 Gemini 的命令
7. 示例 prompt / 示例任务
8. 如何查看状态和 artifacts
9. 常见故障排查

#### 文档风格要求

- 面向开发者使用，不写架构论文
- 一页内先给 happy path
- 后面再给细节和故障排查

---

### 5.3 P0：把“Claude 主管 + 子代理团队”做成默认推荐工作流

#### v0.8 默认推荐架构

- **主会话**：Claude Code
- **主模型建议**：`opusplan`
- **计划/架构升级模式**：必要时切 `opus`
- **Build 子代理**：Codex / Gemini / Ollama
- **Review 子代理**：Codex correctness reviewer + Claude/Gemini style reviewer

#### 为什么这样拍板

因为这条路径天然匹配：

- Claude 适合主管、规划、综合判断
- Codex 适合结构化 build / correctness 审查
- Gemini 适合快研究和前端/UI实现
- Ollama 适合本地 fallback

#### v0.8 必做内容

`claude-opus-supervisor` preset 必须成为最清晰的默认团队，并在 README 中解释每个角色：

- `fast-researcher`
- `backend-coder`
- `frontend-builder`
- `correctness-reviewer`
- `style-reviewer`
- `local-fallback-coder`

---

### 5.4 P0：命令面需要一个“最短路径”

当前命令已经很多，但第一次使用仍有一点分散。

#### v0.8 建议新增

```bash
mcp-subagent quickstart --preset claude-opus-supervisor
```

该命令做以下事情：

1. 若当前目录未初始化，则执行 `init`
2. 执行 `validate`
3. 执行 `doctor`
4. 输出 connect snippets
5. 输出下一步建议

如果不想新增命令，至少也要在 README 里把这个流程固化成三条命令。

---

### 5.5 P0：必须有一个正式的“真实使用示例”

v0.8 必须内置一个面向真实开发场景的示例，不只是一份最小 workflow demo。

#### 建议新增 examples

```text
examples/
  agents/
    workflow_builder.agent.toml
    claude_opus_supervisor_demo/*.agent.toml
  workspaces/
    workflow_demo/
    rust_service_refactor/
    frontend_landing_page/
```

#### 至少两个端到端示例

1. **Rust 后端改造示例**
   - 主会话规划
   - `backend-coder` 实施
   - `correctness-reviewer` 审查
   - `style-reviewer` 审查

2. **前端实现示例**
   - 主会话规划
   - `frontend-builder` 实施
   - `gemini-style-reviewer` 或 `style-reviewer` 审查

---

## 6. P1：代码结构与可维护性收口

### 6.1 `server.rs` 继续拆分

虽然 v0.7 已经比之前更模块化，但 `src/mcp/server.rs` 仍然过大。

#### v0.8 目标

再拆至少一层：

- `server.rs`：只做装配
- `service.rs`：业务入口
- `tools.rs`：MCP tools 定义
- `state.rs`：运行态内存状态
- `persistence.rs`：磁盘读写
- `archive.rs`：归档
- `review.rs`：review pipeline
- `artifacts.rs`：artifact 访问
- `helpers.rs` / `dto.rs`：纯辅助层

#### 原则

`server.rs` 必须收缩成“看得懂入口”的规模。

### 6.2 provider runner 继续统一

目标不是再改大架构，而是保证所有真实 provider 行为尽量走统一 runtime 流程：

- prepare request
- resolve memory
- compile context
- prepare workspace
- acquire conflict policy
- run provider
- parse structured summary
- persist result
- archive / review hooks

尽量避免 provider-specific 快捷分支绕过通用流程。

### 6.3 review role 从启发式变成显式角色

目前 review track 已经能工作，但 v0.8 建议显式化：

- `correctness`
- `style`

而不是主要依赖关键词识别。

agent spec 中建议允许：

```toml
[workflow.review_role]
kind = "correctness"
```

或：

```toml
review_role = "style"
```

这样 dispatcher 与 review policy 会更稳定。

---

## 7. P1：workflow 层继续产品化

### 7.1 保持五阶段模型

继续固定：

- Research
- Plan
- Build
- Review
- Archive

### 7.2 强化 `PLAN.md` 作为一等控制面

`PLAN.md` 不只是 memory source，而是任务合同。

#### v0.8 要求

summary 必须引用：

- `plan_refs`
- `touched_files`
- `verification_status`
- `artifact_index`

并建议补：

- `acceptance_status`
- `rollback_notes`

### 7.3 Archive / Knowledge Capture 继续做实

v0.8 至少要让归档成果更容易被用户发现：

- `docs/plans/...`
- `final_summary.md`
- `decision_note.md`
- metadata index

并且 README / 示例里必须展示一次 archive 后长什么样。

---

## 8. P1：provider 能力策略（明确拍板）

### 8.1 Claude

#### 定位

- 最适合作为**主会话主管**
- 也适合做 style reviewer / 综合判断 reviewer

#### v0.8 使用建议

- 默认推荐主会话模型：`opusplan`
- 架构和高风险设计：升级到 `opus`
- style review：`sonnet`

#### v0.8 注意事项

- Claude 原生 subagents 与 `mcp-subagent` 不冲突，但要明确区分：
  - Claude native subagents：Claude 自己内部 delegation
  - `mcp-subagent` workers：跨 provider 外部 delegation

文档里必须明确这一点，避免用户混淆。

### 8.2 Codex

#### 定位

- v0.8 的首选 Build runner
- correctness reviewer 的首选 provider

#### v0.8 使用建议

- build：`gpt-5.3-codex`（当前 preset）
- correctness review：`gpt-5.3-codex` + higher reasoning effort
- style review：可以使用 Codex，但默认仍优先 Claude/Gemini 风格 reviewer

### 8.3 Gemini

#### 定位

- 快速研究
- 前端/UI实现
- 轻量风格审查

#### v0.8 使用建议

- `fast-researcher`：`flash`
- `frontend-builder`：`pro`
- 简单任务：`flash` / `flash-lite`

#### 注意

文档不再建议 pin 到过于激进的 preview 型号名。统一推荐使用 CLI 可识别别名（如 `pro` / `flash`），减少版本漂移。

### 8.4 Ollama

#### 定位

- 可选本地 fallback
- 无网场景 / 成本敏感场景
- 小步改动、本地实验

#### v0.8 要求

保留为可选路径，但不要把它宣传成与 Codex / Claude / Gemini 同成熟度。

---

## 9. v0.8 默认团队与预设（拍板）

### 9.1 主推荐 preset：`claude-opus-supervisor`

这是 v0.8 的主打预设。

#### 角色定义

##### `fast-researcher`

- Provider：Gemini
- Model：`flash`
- 用途：依赖映射、只读探索、风险扫描
- Stage：Research / Plan
- Sandbox：ReadOnly

##### `backend-coder`

- Provider：Codex
- Model：`gpt-5.3-codex`
- 用途：后端、Rust、服务端改动
- Stage：Build / Review
- Sandbox：WorkspaceWrite

##### `frontend-builder`

- Provider：Gemini
- Model：`pro`
- 用途：前端/UI构建
- Stage：Build / Review
- Sandbox：WorkspaceWrite

##### `correctness-reviewer`

- Provider：Codex
- Model：`gpt-5.3-codex`
- 用途：逻辑正确性、回归风险、验收核对
- Stage：Review
- Sandbox：ReadOnly

##### `style-reviewer`

- Provider：Claude
- Model：`sonnet`
- 用途：命名、可维护性、结构、风格一致性
- Stage：Review / Archive
- Sandbox：ReadOnly

##### `local-fallback-coder`

- Provider：Ollama
- Model：`qwen2.5-coder`
- 用途：本地 fallback / 小步修改
- Stage：Build
- Sandbox：WorkspaceWrite

### 9.2 保留 preset

以下 preset 保留，但不作为 v0.8 主路线：

- `codex-primary-builder`
- `gemini-frontend-team`
- `local-ollama-fallback`
- `minimal-single-provider`

### 9.3 v0.8 可选新增 preset

建议新增两个更贴近真实用户入口的 preset：

#### `claude-opus-supervisor-lite`

只生成：

- `fast-researcher`
- `backend-coder`
- `correctness-reviewer`
- `style-reviewer`

适合绝大多数后端项目。

#### `frontend-fast-lane`

只生成：

- `fast-researcher`
- `frontend-builder`
- `gemini-style-reviewer`

适合页面/UI任务。

---

## 10. v0.8 用户体验目标：安装与接入

### 10.1 安装 `mcp-subagent`

#### 开发安装

```bash
cargo install --git https://github.com/gezihuzi/mcp-subagent-rs mcp-subagent
```

#### 本地开发运行

```bash
cargo run -- mcp
```

#### 正式建议

v0.8 发布时要提供 GitHub Release 二进制下载说明。

### 10.2 安装主客户端（文档层给出推荐，不内嵌管理）

#### Claude Code

优先推荐原生安装，不再推荐老的 npm 路径。

#### Codex CLI

按官方方式安装并登录。

#### Gemini CLI

按官方方式安装并登录。

### 10.3 v0.8 文档必须明确：先装哪个最简单

推荐顺序：

1. Claude Code（主客户端）
2. Codex CLI（最重要 worker）
3. Gemini CLI（可选前端 / research worker）
4. Ollama（可选本地 fallback）

---

## 11. v0.8 详细使用示例（必须写进最终 README / docs）

---

### 11.1 最短本地体验（Standalone）

```bash
# 1) 初始化
mcp-subagent init --preset claude-opus-supervisor

# 2) 校验配置
mcp-subagent validate --agents-dir ./agents

# 3) 检查本地 provider 准备情况
mcp-subagent doctor --agents-dir ./agents

# 4) 查看可用 agents
mcp-subagent list-agents --agents-dir ./agents

# 5) 直接本地跑一个 research 任务
mcp-subagent run fast-researcher \
  --agents-dir ./agents \
  --task "Map this repository and identify the top 5 risk areas before refactoring." \
  --stage Research \
  --working-dir .
```

### 11.2 最短主推荐路径：Claude Code 作为主管

#### 第一步：初始化工作区

```bash
mcp-subagent init --preset claude-opus-supervisor
mcp-subagent validate --agents-dir ./agents
mcp-subagent doctor --agents-dir ./agents
```

#### 第二步：把 `mcp-subagent` 接到 Claude Code

```bash
claude mcp add --transport stdio mcp-subagent -- \
  /ABS/PATH/TO/mcp-subagent \
  --agents-dir /ABS/PATH/TO/PROJECT/agents \
  --state-dir /ABS/PATH/TO/PROJECT/.mcp-subagent/state \
  mcp
```

#### 第三步：进入 Claude Code 会话

在项目目录里启动 Claude Code。

建议主模型：

- 日常默认：`/model opusplan`
- 重型架构：`/model opus`

#### 第四步：对 Claude 说

```text
使用 mcp-subagent 先列出当前 agents。
接着基于当前仓库生成或更新 PLAN.md。
然后把后端实现工作派给 backend-coder，把只读调研交给 fast-researcher。
等结果回来后，再让 correctness-reviewer 和 style-reviewer 分别做审查。
不要把原始日志塞回主线程，只吸收结构化 summary 和 artifacts。
```

### 11.3 Claude + 子代理的实际分工建议

#### 主管 Claude 负责

- 目标定义
- 约束整理
- 计划审查
- 最终判断
- 结果合并

#### `mcp-subagent` 子代理负责

- 大量只读探索
- 编码实现
- 独立审查
- 归档总结

### 11.4 Codex 作为主客户端接入

```bash
codex mcp add mcp-subagent -- \
  /ABS/PATH/TO/mcp-subagent \
  --agents-dir /ABS/PATH/TO/PROJECT/agents \
  --state-dir /ABS/PATH/TO/PROJECT/.mcp-subagent/state \
  mcp
```

在 Codex 中让它：

```text
Use the mcp-subagent tools to inspect the team, update PLAN.md, then delegate build work to backend-coder and review work to correctness-reviewer.
Keep the main thread clean and rely on structured summaries.
```

### 11.5 Gemini 作为主客户端接入

```bash
gemini mcp add mcp-subagent \
  /ABS/PATH/TO/mcp-subagent \
  --agents-dir /ABS/PATH/TO/PROJECT/agents \
  --state-dir /ABS/PATH/TO/PROJECT/.mcp-subagent/state \
  mcp
```

在 Gemini 中建议主模型使用 Auto 或 Pro，然后让它通过 MCP 调 `mcp-subagent` 进行分工。

### 11.6 用配置文件方式接入 Codex（推荐给稳定用户）

`~/.codex/config.toml`：

```toml
[mcp_servers.mcp_subagent]
command = "/ABS/PATH/TO/mcp-subagent"
args = [
  "--agents-dir", "/ABS/PATH/TO/PROJECT/agents",
  "--state-dir", "/ABS/PATH/TO/PROJECT/.mcp-subagent/state",
  "mcp"
]
cwd = "/ABS/PATH/TO/PROJECT"
```

### 11.7 用配置文件方式接入 Gemini（推荐给稳定用户）

`settings.json`：

```json
{
  "mcpServers": {
    "mcp-subagent": {
      "command": "/ABS/PATH/TO/mcp-subagent",
      "args": [
        "--agents-dir", "/ABS/PATH/TO/PROJECT/agents",
        "--state-dir", "/ABS/PATH/TO/PROJECT/.mcp-subagent/state",
        "mcp"
      ],
      "cwd": "/ABS/PATH/TO/PROJECT",
      "trust": true,
      "timeout": 30000
    }
  }
}
```

---

## 12. v0.8 配置建议（针对你的实际场景拍板）

### 12.1 主推荐场景

> Claude Code 使用 `opusplan` 作为主管。  
> `backend-coder` 使用 Codex 负责后端 / Rust / 常规编码。  
> `frontend-builder` 使用 Gemini `pro` 负责前端 / UI。  
> `correctness-reviewer` 用 Codex。  
> `style-reviewer` 用 Claude `sonnet`。  
> `fast-researcher` 用 Gemini `flash`。  
> `local-fallback-coder` 用 Ollama 兜底。

### 12.2 为什么这样默认

因为这条路线同时照顾了：

- 主管模型的综合判断力
- 实施模型的成本控制
- 把大 token 消耗留给高价值阶段
- 用更便宜/更快模型做边界清晰的子任务
- review 分轨，减少错误回流到主线程

### 12.3 主管模型的实践建议

#### 默认

- `opusplan`

#### 什么时候切纯 `opus`

- 系统级架构设计
- 大范围重构路线选择
- 多方案权衡
- 涉及安全、数据一致性、回滚策略

#### 什么时候切回更轻模型

- 已有明确 `PLAN.md`
- 任务开始进入执行和验收阶段
- 主管更多在做“调度 + 审核”，不是“从零推理”

### 12.4 Worker 默认策略

#### `backend-coder`

```toml
[core]
provider = "Codex"
model = "gpt-5.3-codex"

[runtime]
context_mode = { SelectedFiles = ["src/**", "Cargo.toml", "PLAN.md"] }
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "WorkspaceWrite"
approval = "DenyByDefault"
spawn_policy = "Async"
```

#### `frontend-builder`

```toml
[core]
provider = "Gemini"
model = "pro"

[runtime]
context_mode = { SelectedFiles = ["web/**", "src/**", "package.json", "PLAN.md"] }
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "WorkspaceWrite"
approval = "ProviderDefault"
spawn_policy = "Async"
```

#### `correctness-reviewer`

```toml
[core]
provider = "Codex"
model = "gpt-5.3-codex"

[runtime]
context_mode = "SummaryOnly"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
sandbox = "ReadOnly"
approval = "DenyByDefault"
spawn_policy = "Sync"
```

#### `style-reviewer`

```toml
[core]
provider = "Claude"
model = "sonnet"

[runtime]
context_mode = "SummaryOnly"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
sandbox = "ReadOnly"
approval = "DenyByDefault"
spawn_policy = "Sync"
```

#### `fast-researcher`

```toml
[core]
provider = "Gemini"
model = "flash"

[runtime]
context_mode = "ExpandedBrief"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
sandbox = "ReadOnly"
spawn_policy = "Sync"
```

---

## 13. v0.8 测试与验收要求

### 13.1 自动化测试

必须覆盖：

- context mode
- workflow gate
- runtime depth
- workspace auto / git worktree fallback / cleanup
- provider flag mapping
- summary parse
- archive output
- init presets
- connect snippet generation

### 13.2 本地 smoke

新增：

```bash
./scripts/smoke_v08.sh
```

至少验证：

1. `validate`
2. `doctor`
3. `list-agents`
4. mock run
5. codex fake runner run
6. `mcp` boot
7. `connect-snippet --host claude`
8. `connect-snippet --host codex`
9. `connect-snippet --host gemini`

### 13.3 手工验收矩阵

至少三条手工验收：

- Claude host + Codex worker
- Claude host + Gemini worker
- Standalone run + artifact readback

---

## 14. v0.8 需要修的已知问题（拍板清单）

### P0

- [ ] 生成**可直接复制**的 host 接入命令
- [ ] README 模板升级成真正 onboarding 文档
- [ ] 发布首个 GitHub Release 二进制
- [ ] 强化 `claude-opus-supervisor` preset 文档
- [ ] 新增至少两个真实示例工作区
- [ ] 新增 `smoke_v08.sh`
- [ ] 文档与命令面完全对齐（不允许历史版本漂移）

### P1

- [ ] 继续拆小 `server.rs`
- [ ] review role 显式化
- [ ] 增加 `quickstart` 或 `connect-snippet`
- [ ] 增加 `runs list` / `watch` / `logs` 之类更直观的辅助命令（可选）
- [ ] 增加更多 preset

### P2

- [ ] 后续再考虑 HTTP transport
- [ ] 后续再考虑更细粒度冲突锁
- [ ] 后续再考虑 provider capability matrix 输出给用户

---

## 15. 最终拍板

### 对 v0.7 的定性

v0.7 已经把底层系统基本做对了：

- runtime 能跑
- workflow 已落地
- 多 provider 已成型
- preset 已具备
- 主推荐场景也已经有雏形

### v0.8 的唯一主题

> **不是继续证明它能做什么，而是让一个正常开发者第一次就能把它用起来。**

### 最后一句话的版本目标

v0.8 发布后，用户应能做到：

1. 安装 `mcp-subagent`
2. `init --preset claude-opus-supervisor`
3. 复制一条 Claude/Codex/Gemini 接入命令
4. 在主会话中把任务派给跨 provider 子代理
5. 在不污染主线程的情况下完成计划、执行、审查和归档

如果这个体验没有做到，v0.8 就还不算完成。

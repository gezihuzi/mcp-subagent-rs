# mcp-subagent 技术设计与使用文档 v0.7

**项目名称**：`mcp-subagent`  
**仓库名**：`mcp-subagent-rs`  
**文档定位**：代码审计结论 + 下一阶段实现基线 + 本地开发/接入/协作使用手册  
**适用对象**：你本人、实现同事、团队评审、未来使用该 runtime 的开发者

---

## 0. 一句话结论

当前仓库已经不是“原型草图”，而是一个**可继续演进的本地多 LLM subagent runtime 雏形**。

它已经具备：

- 分层 spec（`core / runtime / provider_overrides / workflow`）
- 真正生效的上下文编译器与 summary contract
- 多 provider runner 抽象（Claude / Codex / Gemini / Ollama / Mock）
- workspace 隔离、清理、序列化冲突控制
- 持久化 run state / artifacts / summary / compiled context
- MCP server（stdio）和本地 CLI 命令面
- workflow 层（Research / Plan / Build / Review / Archive）
- ActivePlan / archived plans / provider-native memory 处理

**最终判断**：

- **可以继续开发，不需要推翻重来。**
- 下一阶段重点不是“再加很多新功能”，而是：
  1. 修正 provider flag 映射的已知问题。  
  2. 把 partial 实现明确收口。  
  3. 把默认团队配置和使用方式产品化。  
  4. 围绕你的实际场景，提供一个“Claude 主管 + Codex/Gemini 子代理”的可直接使用预设。

---

## 1. 本版文档的拍板目标

本版 v0.7 做三件事：

1. **审核当前实现**，给出可执行的 P0 / P1 / P2 结论。  
2. **拍板你的目标工作流**：Claude Code 主代理 + 其他模型子代理协作。  
3. **给出直接可实现、可本地跑的下一阶段规范**，包括：
   - 安装与接入方式
   - 开发者如何使用
   - 默认 agent 配置
   - 默认团队预设
   - 对当前代码的修复清单

---

## 2. 当前实现状态：已完成、部分完成、待修复

### 2.1 已完成并且方向正确

#### A. spec 已经收口为可维护结构

当前代码已经不是一个巨型 `AgentSpec` 垃圾桶，而是：

- `AgentSpecCore`
- `RuntimePolicy`
- `ProviderOverrides`
- `WorkflowSpec`

这是对的，应该继续保持。

#### B. `context_mode` 已真正生效

`ContextMode` 不再只是 schema 字段，而是已经在 `runtime/context.rs` 里驱动上下文注入行为：

- `Isolated`
- `SummaryOnly`
- `SelectedFiles`
- `ExpandedBrief`

并且已经有：

- parent summary 注入控制
- raw transcript 识别与抑制
- selected file allowlist 控制
- provider native memory passthrough 标记

这解决了上一轮最重要的设计-实现落差。

#### C. workflow 层已经进入代码

你们已经把统一工作流从“建议”落到了代码：

- `Research`
- `Plan`
- `Build`
- `Review`
- `Archive`

并支持：

- `WorkflowGatePolicy`
- `ActivePlan`
- `ReviewPolicy`
- `KnowledgeCapturePolicy`
- `ArchivePolicy`
- `max_runtime_depth`

这是非常对的，因为多 agent 真正难的是“围绕什么工件协作”，不是单纯 spawn 进程。

#### D. workspace 隔离和清理已经像生产代码

当前已支持：

- `InPlace`
- `TempCopy`
- `GitWorktree`
- `Auto`

并且有：

- `GitWorktree` 失败时 fallback 到 `TempCopy`
- cleanup guard 自动清理临时 workspace / worktree

这是 runtime 长期可用的关键能力。

#### E. runner 抽象已统一

当前 `runtime/runners` 已统一到真正的 `AgentRunner` 抽象，而不是早期那种“Mock 用抽象，真实 provider 走分支”的双轨实现。这一步已经做对了。

#### F. MCP 层已经从巨型文件走向模块化

现在 `mcp` 已拆成：

- `server`
- `tools`
- `service`
- `state`
- `persistence`
- `artifacts`
- `dto`

虽然 `server.rs` 仍偏大，但已经比上一阶段健康很多。

#### G. 你的目标协作模式已经具备技术基础

你想要的是：

- Claude 主管（高能力、顶层规划）
- Codex / Gemini 作为子代理执行子任务
- 子代理带着结构化结果回到主管
- 降低主线程 token 消耗
- 降低 context pollution
- 强化 plan / memory / archive

当前代码已经具备这个模式需要的内核：

- 子代理隔离上下文
- summary contract
- stage / plan / memory
- artifact index
- run state persistence

也就是说，你现在差的不是“能不能做”，而是“怎么更顺手、更稳定、更默认化”。

---

### 2.2 部分完成： schema 已到位，但执行尚未完全兑现

#### A. `spawn_policy` / `background_preference`

当前 schema 已定义，但实际前后台行为仍主要由调用入口决定：

- `run_agent` = 同步
- `spawn_agent` = 异步

也就是说，spec 中的这些字段还没有真正主导执行路径。

#### B. `max_turns`

当前已在 schema 中存在并校验，但尚未真正下沉到各 provider runner。

#### C. `retry_policy`

当前定义了，但还没有形成真正的 provider / dispatch retry 行为。

#### D. workflow gate 的部分条件未执行

当前 `WorkflowGatePolicy` 比实际 enforcement 更丰富。已实际落地的主要是：

- touched files threshold
- estimated runtime threshold
- parallel agents threshold

但以下条件目前更多还是“声明式字段”，还未被真正推理或执行：

- `require_plan_if_cross_module`
- `require_plan_if_new_interface`
- `require_plan_if_migration`
- `require_plan_if_human_approval_point`

#### E. ReviewPolicy / KnowledgeCapturePolicy / ArchivePolicy 仍偏 schema-first

当前结构已经有了，但 stage-aware routing、强制双审、自动经验沉淀、计划归档写入等，仍未完全闭环。

---

### 2.3 必须优先修复的 P0 问题

#### P0-1. Gemini approval mode 映射存在明显不一致风险

当前 Gemini runner 把 `ReadOnly` 映射为 `--approval-mode plan`。  
**建议直接修正为：**

- `ReadOnly` -> `default`
- `WorkspaceWrite` -> `auto_edit`
- `FullAccess` -> `yolo`

并把这套映射写进 probe/doctor 输出。

#### P0-2. Claude permission mode 映射需要按当前公开模式重新校准

当前 Claude runner 允许：

- `plan`
- `acceptEdits`
- `auto`

建议改成基于当前公开模式：

- `default`
- `acceptEdits`
- `plan`
- `dontAsk`
- `bypassPermissions`

如果你们本地已验证某个额外私有/兼容模式仍可用，也必须在代码中标明“版本锁定假设”，不能把它当成公共稳定接口。

#### P0-3. 当前二进制 MCP 启动方式与旧文档不一致

当前实现实际命令是：

```bash
mcp-subagent mcp
```

而不是早期文档中的：

```bash
mcp-subagent --mcp
```

这不是大问题，但文档和接入示例必须统一，否则主机端配置会错。

#### P0-4. 当前服务传输层是 stdio-first，不应继续宣称 HTTP 已就绪

当前代码已经实现的是 **stdio MCP server**。  
因此文档必须明确：

- 当前已实现：stdio
- HTTP：未来项 / 未实现

#### P0-5. 子代理“不能欺骗”不能靠口头要求，必须靠可核验结果

没有任何 runtime 能保证 LLM **绝不欺骗 / 绝不幻觉**。  
能做的是：

- 限制上下文来源
- 强制 summary contract
- 要求 plan 引用
- 要求 artifact / touched_files / verification_status
- 要求 reviewer 二次核验
- 在主代理侧默认不接受“无证据的通过”

这条必须写进下一版规范。

---

### 2.4 P1 改进项

#### P1-1. `mcp/server.rs` 仍偏大

它已经不是灾难级别，但仍然超过 1000 行。建议继续向下沉：

- provider capability 说明
- snapshot 构造
- 输入输出映射
- run 辅助函数

#### P1-2. CLI `--selected-file` 当前只传路径，不传内容

这意味着本地 `run` / `spawn` 命令当前只会把文件路径转成 selected file 输入，而不会自动 inline 文件内容。

这不是错误，但意味着：

- 目前 selected_files 更像“显式指路”
- 不像“强制内联上下文”

建议未来增加两种模式：

- `--selected-file path`：仅传路径
- `--selected-file-inline path`：读取文件内容并内联传入

#### P1-3. `ReadOnly + GitWorktree` 的校验较保守

当前校验直接禁止这组组合。它很安全，但也限制了“只读且隔离”的分析场景。  
建议保留现状作为默认策略，但在后续版本考虑加入：

- `ReadOnly + GitWorktree` 允许，但仅限 Research / Plan stage

#### P1-4. Ollama 现在不再只是占位，但仍不应在文档里装作一等主路径

当前已经有真实 Ollama runner，这很好。  
但相对 Claude / Codex / Gemini，它仍应在文档里标为：

- local/community path
- optional local fallback
- not default primary preset

---

## 3. 最终产品方向：从 Agent Runtime 升级为 Agent Workflow Runtime

本项目下一阶段不应只被定义为：

> 一个能调起多个 provider CLI 的 runtime。

而应被定义为：

> 一个本地可运行的、以 plan / summary / artifacts 为核心工件的多 LLM 工作流 runtime。

也就是说：

- **runtime** 负责调度
- **workflow** 负责秩序
- **plan** 负责共享事实
- **summary** 负责回传
- **archive** 负责长期积累

这也是最符合你场景的路线。

---

## 4. 拍板后的核心原则（v0.7 起视为强约束）

### 4.1 上下文原则

1. Runtime **MUST NOT** forward raw parent transcript to child agents.  
2. 子代理只允许接收：
   - task
   - task_brief
   - parent_summary（按 mode 控制）
   - selected_files
   - plan refs
   - memory sources
3. provider-native memory（如 `CLAUDE.md` / `AGENTS.md` / `GEMINI.md`）默认走 native discovery，不重复 inline。  
4. ActivePlan 属于高优先级共享事实层。  
5. archived plans 属于低优先级历史经验层。  
6. 主管默认不得把“整段聊天记录”灌给子代理。

### 4.2 结果可信度原则

1. 子代理的“完成”不等于真实完成。  
2. 所有 Build / Review 结果必须带：
   - `summary`
   - `verification_status`
   - `touched_files`
   - `plan_refs`
   - `exit_code`
3. 主代理默认**不应信任无证据声称**。至少满足以下之一才应接受：
   - patch / diff artifact
   - touched_files 非空且合理
   - stdout/stderr 有可核验输出
   - reviewer 二次确认
4. `verification_status = Passed` 时，必须伴随一句明确说明：
   - 跑了什么
   - 还是仅静态检查
   - 有何局限

### 4.3 多 agent 协作原则

1. 80% 的质量来自 plan，不来自更多子代理。  
2. Build 前优先有 `PLAN.md`。  
3. 子代理围绕 plan section 工作，不围绕主线程自由发挥。  
4. 写代码任务默认隔离 workspace。  
5. 高风险任务默认双审。  
6. 经验沉淀不依赖“人想起来再记”，而应由 workflow hook 或明确 Archive stage 触发。

---

## 5. 标准统一工作流（拍板版）

### 5.1 五阶段

#### Stage 1: Research

目标：收集事实，不改代码。  
约束：只读。  
产物：

- affected areas
- 风险列表
- 候选方案
- 待确认问题

#### Stage 2: Plan

目标：把任务转成结构化执行计划。  
约束：不大规模改代码。  
产物：`PLAN.md`

#### Stage 3: Build

目标：按 plan 的 section 落地变更。  
约束：必须引用 plan。  
产物：

- 变更结果
- patch / diff
- touched_files
- verification_status

#### Stage 4: Review

目标：验证 Build 结果。  
建议拆成：

- correctness review
- style / maintainability review

#### Stage 5: Archive

目标：沉淀可复用知识。  
产物：

- archived plan
- final summary
- decision note（如需要）
- metadata index

---

### 5.2 Plan 强制门槛（最终建议）

满足任一条件时，Build / Review 前 **必须有 `PLAN.md`**：

- 预期修改 >= 5 个文件
- 跨模块 / 跨 crate
- 需要并行子代理
- 需要新接口 / 新配置 / schema 变更
- 需要迁移
- 需要人工审批点
- timeout / 任务时长预估较高

当前代码已部分实现，下一阶段应把其余 gate 条件补齐。

---

### 5.3 `PLAN.md` 模板（建议固定）

```md
# PLAN

## Goal

## Scope

## Non-goals

## Constraints

## Affected areas

## Research findings

## Execution steps
1.
2.
3.

## Acceptance criteria
1.
2.
3.

## Rollback / fallback

## Review checklist

## Artifacts to produce

## Final summary
```

建议后续加 frontmatter：

```yaml
---
id: plan-2026-03-24-backend-refactor
status: active
repo: mcp-subagent-rs
owners: ["you"]
agents: ["backend-coder", "correctness-reviewer"]
tags: ["runtime", "workflow", "mcp"]
created_at: 2026-03-24
---
```

---

## 6. 针对你场景的默认团队设计（拍板版）

你的目标场景不是“所有模型都平权”，而是：

- **Claude Code 主会话**负责：
  - 任务理解
  - 顶层设计
  - 风险权衡
  - 委派
  - 审批
  - 汇总
- **mcp-subagent** 负责：
  - 以统一工具面暴露子代理
  - 调度 Codex / Gemini / Claude / Ollama runner
  - 负责工作流边界、上下文边界、summary contract、artifact 持久化
- **子代理**负责：
  - 某一类具体子任务
  - 带结果回来，不接管整体方向

### 6.1 主管模型建议

#### 默认建议：Claude Code 主会话使用 `opusplan`

原因：

- 你希望顶层规划尽量强
- 同时又希望执行阶段不要一直用最重模型烧 token
- `opusplan` 本身就是 Claude 官方提供的“Plan 用 Opus、执行用 Sonnet”的混合别名

#### 需要最高级架构判断时：使用 `opus`

适合：

- 架构评审
- 跨模块重构方案
- 风险分析
- 技术路线选择

#### 日常一般任务：可以直接 `sonnet`

适合：

- review
- 小改动
- 轻量研究

### 6.2 子代理角色建议

#### 1. `backend-coder`（Codex）

适合：

- Rust / 后端实现
- 测试补全
- 常规业务逻辑修改
- 中等复杂度 refactor

#### 2. `frontend-builder`（Gemini）

适合：

- Web 前端实现
- 交互界面落地
- 布局与组件变更
- 高速 UI patch

#### 3. `fast-researcher`（Gemini Flash 或 Claude Sonnet/Haiku 风格轻量 agent）

适合：

- 只读研究
- 代码库扫描
- 文件关系梳理
- 风险点初筛

#### 4. `correctness-reviewer`（Codex）

适合：

- 边界条件检查
- 回归风险检查
- 测试覆盖建议
- 逻辑一致性审核

#### 5. `style-reviewer`（Claude Sonnet）

适合：

- 可维护性
- 命名与结构
- 文档性
- 团队风格一致性

#### 6. `local-fallback-coder`（Ollama，可选）

适合：

- 完全本地实验
- 非关键实现
- 离线 fallback

---

## 7. 模型路由默认策略（拍板版）

### 7.1 顶层策略

| 任务类型 | 默认主模型 / 子模型 | 原因 |
|---|---|---|
| 顶层方案、架构取舍、复杂设计 | Claude `opus` / `opusplan` | 主管看全局、少做具体实现 |
| 后端实现、通用编码、测试 | Codex `gpt-5.3-codex` | 强实现能力，适合作为便宜于 Opus 的执行层 |
| 前端页面与 UI 功能实现 | Gemini `pro` 或 `flash` | Gemini 在前端/快速迭代场景可作为强有力执行代理 |
| 只读研究、扫描、快速定位 | Gemini `flash` 或 Claude 轻量 agent | 速度优先 |
| correctness review | Codex / Claude Sonnet | 静态核验和代码推理 |
| 风格与可维护性 review | Claude Sonnet | 文风与代码可读性把关 |
| 完全本地 fallback | Ollama | 无外部服务依赖 |

### 7.2 默认规则

1. **主代理尽量少做具体编码。**  
2. **执行型任务优先丢给子代理。**  
3. **研究型任务优先用更快模型。**  
4. **review 不与 build 共用同一个子代理结果作为唯一真相。**  
5. **高风险任务尽量 correctness reviewer + style reviewer 双审。**

---

## 8. 推荐预设：`claude-opus-supervisor` 团队

这是我为你的实际场景拍板的默认团队。

### 8.1 团队结构

- 主会话：Claude Code，模型 `opusplan`（默认）或 `opus`
- 子代理：
  - `fast-researcher` -> Gemini Flash
  - `backend-coder` -> Codex
  - `frontend-builder` -> Gemini Pro / Auto (Gemini 3)
  - `correctness-reviewer` -> Codex
  - `style-reviewer` -> Claude Sonnet
  - `local-fallback-coder` -> Ollama（可选）

### 8.2 默认流程

1. 主代理先 Research。  
2. 复杂任务必须生成 / 更新 `PLAN.md`。  
3. Build 任务按 affected areas 分发给 Codex / Gemini。  
4. 所有子代理 summary 必须引用 plan step。  
5. correctness reviewer 审逻辑和回归。  
6. style reviewer 审结构和可维护性。  
7. 主代理再整合并决定是否归档与沉淀。

### 8.3 默认安全/抗偏移策略

- `max_runtime_depth = 1`  
- Build 默认 `GitWorktree` / `Auto -> GitWorktree`  
- `file_conflict_policy = Serialize`  
- `context_mode` 默认不传 raw transcript  
- `ActivePlan` 默认开启  
- `ArchivedPlans` 默认按需开启  
- `verification_status = Passed` 必须附验证说明  
- 主代理必须在 merge / 最终接受前至少检查：
  - `touched_files`
  - `open_questions`
  - `artifact_index`
  - `stdout` / `stderr` / diff

---

## 9. 当前实现到 v0.7 的修复清单

### 9.1 P0（必须先做）

1. **修 Gemini approval mode 映射**  
   - 把 `plan` 去掉  
   - 改成 `default / auto_edit / yolo`

2. **修 Claude permission mode 映射**  
   - 默认使用公开模式名  
   - provider override 允许值与文档对齐

3. **统一所有文档与接入示例到实际命令面**  
   - 当前实际命令：`mcp-subagent mcp`

4. **文档和 README 明确当前只支持 stdio MCP server**

5. **在 README / docs 中明确“不能保证零幻觉，只能提高可核验性”**

### 9.2 P1（下一阶段）

1. 增加 `init` 命令：
   - `mcp-subagent init --preset claude-opus-supervisor`
   - 自动生成 `agents/`、`PLAN.md` 模板、配置文件、接入说明

2. 增加 `--selected-file-inline`

3. 将 workflow gate 的其余字段真正落地

4. stage-aware routing：
   - Plan / Research stage 不应被任意 agent 乱用
   - Review stage 应优先 reviewer agent

5. 自动归档 hook：
   - 生成 final summary
   - decision note
   - metadata index

### 9.3 P2（增强）

1. 项目级 agent preset 市场 / preset 包  
2. 更细粒度 file conflict lock  
3. `doctor --json` 与 IDE 友好输出  
4. 更强的 provider version pin / compatibility report

---

## 10. 当前本地版本的命令面（以现实现为准）

当前二进制命令面应以**已实现代码**为准：

```bash
mcp-subagent doctor
mcp-subagent validate
mcp-subagent list-agents
mcp-subagent run <agent> --task "..."
mcp-subagent spawn <agent> --task "..."
mcp-subagent status <handle_id>
mcp-subagent cancel <handle_id>
mcp-subagent artifact <handle_id> --path summary.json
mcp-subagent mcp
```

### 10.1 推荐本地安装方式

#### 开发期

```bash
cargo build --release
./target/release/mcp-subagent doctor --agents-dir ./agents
./target/release/mcp-subagent validate --agents-dir ./agents
```

#### 本地安装

```bash
cargo install --path .
mcp-subagent doctor --agents-dir ./agents
```

### 10.2 推荐目录布局

```text
repo/
├── agents/
│   ├── backend-coder.agent.toml
│   ├── frontend-builder.agent.toml
│   ├── fast-researcher.agent.toml
│   ├── correctness-reviewer.agent.toml
│   ├── style-reviewer.agent.toml
│   └── local-fallback-coder.agent.toml
├── PLAN.md
├── PROJECT.md
├── CLAUDE.md          # 如果主会话/子代理需要 Claude native memory
├── AGENTS.md          # 如果 Codex child 需要 native memory
├── GEMINI.md          # 如果 Gemini child 需要 native memory
└── .mcp-subagent/
    └── state/
```

---

## 11. 如何接入 Claude Code / Codex CLI / Gemini CLI

> 注意：下面示例全部以**当前实现的实际命令**为准，即 `mcp-subagent mcp`。

### 11.1 接入 Claude Code

```bash
claude mcp add --transport stdio mcp-subagent -- \
  /absolute/path/to/mcp-subagent \
  --agents-dir /absolute/path/to/repo/agents \
  --state-dir /absolute/path/to/repo/.mcp-subagent/state \
  mcp
```

接入后，在 Claude Code 里让主代理使用 `mcp-subagent` 暴露的工具：

- `list_agents`
- `run_agent`
- `spawn_agent`
- `get_agent_status`
- `cancel_agent`
- `read_agent_artifact`

### 11.2 接入 Codex CLI

```bash
codex mcp add mcp-subagent -- \
  /absolute/path/to/mcp-subagent \
  --agents-dir /absolute/path/to/repo/agents \
  --state-dir /absolute/path/to/repo/.mcp-subagent/state \
  mcp
```

### 11.3 接入 Gemini CLI

```bash
gemini mcp add mcp-subagent \
  /absolute/path/to/mcp-subagent \
  --agents-dir /absolute/path/to/repo/agents \
  --state-dir /absolute/path/to/repo/.mcp-subagent/state \
  mcp
```

### 11.4 手动配置 Codex `config.toml`

```toml
[mcp_servers.mcp-subagent]
command = "/absolute/path/to/mcp-subagent"
args = [
  "--agents-dir", "/absolute/path/to/repo/agents",
  "--state-dir", "/absolute/path/to/repo/.mcp-subagent/state",
  "mcp"
]
cwd = "/absolute/path/to/repo"
startup_timeout_sec = 15
tool_timeout_sec = 120
```

### 11.5 手动配置 Gemini `.gemini/settings.json`

```json
{
  "mcpServers": {
    "mcp-subagent": {
      "command": "/absolute/path/to/mcp-subagent",
      "args": [
        "--agents-dir", "/absolute/path/to/repo/agents",
        "--state-dir", "/absolute/path/to/repo/.mcp-subagent/state",
        "mcp"
      ],
      "cwd": "/absolute/path/to/repo",
      "timeout": 30000,
      "trust": false
    }
  }
}
```

---

## 12. 开发者如何用（直接操作版）

### 12.1 首次检查环境

```bash
mcp-subagent doctor --agents-dir ./agents
mcp-subagent validate --agents-dir ./agents
mcp-subagent list-agents
```

### 12.2 先研究

```bash
mcp-subagent run fast-researcher \
  --task "梳理 runtime/context.rs、runtime/memory.rs、mcp/tools.rs 的关系，并指出改动风险" \
  --stage Research \
  --working-dir .
```

### 12.3 生成或更新 `PLAN.md`

由主代理或人工根据 research 结果生成 `PLAN.md`。

### 12.4 启动 Build 子代理

```bash
mcp-subagent spawn backend-coder \
  --task "按照 PLAN.md 的步骤 2 与 3 实现 context-mode 与 selected-file-inline 支持" \
  --stage Build \
  --plan PLAN.md \
  --selected-file src/runtime/context.rs \
  --selected-file src/main.rs \
  --working-dir .
```

### 12.5 查看状态

```bash
mcp-subagent status <handle_id>
```

### 12.6 读取结构化结果或日志

```bash
mcp-subagent artifact <handle_id> --kind summary
mcp-subagent artifact <handle_id> --kind log
```

### 12.7 启动 Review

```bash
mcp-subagent run correctness-reviewer \
  --task "审核 backend-coder 的实现是否符合 PLAN.md，重点看边界条件、回归风险和遗漏测试" \
  --stage Review \
  --plan PLAN.md \
  --working-dir .
```

---

## 13. 给 Claude Code 主代理的推荐提示词模板

下面这段是你最值得直接用的主管提示词模板。

```text
你是当前项目的主管代理。

你有一个可用的 MCP 服务：mcp-subagent。
请按以下规则工作：

1. 先判断任务是否需要进入 Research -> Plan -> Build -> Review -> Archive 工作流。
2. 如果任务复杂、跨模块、涉及多个文件或需要并行子代理，先生成或更新 PLAN.md。
3. 不要把当前主会话的原始 transcript 直接传给子代理。
4. 调用 mcp-subagent 时：
   - Research 阶段优先 fast-researcher
   - 后端/通用实现优先 backend-coder
   - 前端/UI 优先 frontend-builder
   - 逻辑核验优先 correctness-reviewer
   - 风格和可维护性核验优先 style-reviewer
5. 子代理完成后，必须检查：
   - structured_summary.summary
   - structured_summary.verification_status
   - structured_summary.touched_files
   - artifact_index
6. 若结果缺少证据，不要直接接受；应继续追问或发起 reviewer 审核。
7. 高风险变更默认双审。
8. 重要变更结束后，归档最终计划和结论。
```

---

## 14. 推荐默认 agent 配置（你的场景）

> 以下示例全部使用当前代码的 `[core] / [runtime] / [provider_overrides] / [workflow]` 结构。

### 14.1 `fast-researcher.agent.toml`

```toml
[core]
name = "fast-researcher"
description = "Fast read-only codebase investigator for targeted research, dependency mapping, and risk discovery."
provider = "Gemini"
model = "flash"
instructions = """
You are a fast read-only research specialist.
Do not edit files.
Your job is to map affected areas, dependencies, risks, and unresolved questions.
Always return concise, evidence-oriented structured summaries.
"""
tags = ["research", "read-only", "fast"]

[runtime]
context_mode = "ExpandedBrief"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "ReadOnly"
approval = "ProviderDefault"
timeout_secs = 600
spawn_policy = "Sync"

[workflow]
enabled = true
stages = ["Research", "Plan"]
max_runtime_depth = 1
```

### 14.2 `backend-coder.agent.toml`

```toml
[core]
name = "backend-coder"
description = "Implements backend, Rust, tests, and common product code changes from an approved plan."
provider = "Codex"
model = "gpt-5.3-codex"
instructions = """
You implement approved changes against PLAN.md.
Stay within scope.
Reference the relevant plan step in your summary.
If uncertain, ask via open_questions instead of inventing certainty.
Prefer minimal, reviewable diffs.
"""
tags = ["build", "backend", "rust", "codex"]

[runtime]
context_mode = { SelectedFiles = ["src/**", "Cargo.toml", "PLAN.md"] }
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "WorkspaceWrite"
approval = "DenyByDefault"
timeout_secs = 1200
spawn_policy = "Async"

[provider_overrides.codex]
model_reasoning_effort = "medium"

[workflow]
enabled = true
stages = ["Build", "Review"]
max_runtime_depth = 1
```

### 14.3 `frontend-builder.agent.toml`

```toml
[core]
name = "frontend-builder"
description = "Implements frontend, UI, and web interaction changes from an approved plan."
provider = "Gemini"
model = "pro"
instructions = """
You implement frontend/UI changes from PLAN.md.
Focus on usability, correctness, and minimal diffs.
When uncertain, explicitly list assumptions and open questions.
"""
tags = ["build", "frontend", "ui", "gemini"]

[runtime]
context_mode = { SelectedFiles = ["web/**", "src/**", "package.json", "PLAN.md"] }
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "WorkspaceWrite"
approval = "ProviderDefault"
timeout_secs = 1200
spawn_policy = "Async"

[provider_overrides.gemini]
experimental_subagents = true

[workflow]
enabled = true
stages = ["Build", "Review"]
max_runtime_depth = 1
```

### 14.4 `correctness-reviewer.agent.toml`

```toml
[core]
name = "correctness-reviewer"
description = "Reviews logic, regressions, edge cases, and verification claims."
provider = "Codex"
model = "gpt-5.3-codex"
instructions = """
You are a correctness reviewer.
Do not rewrite large sections unless necessary.
Audit claims, regression risk, verification gaps, and plan compliance.
If evidence is missing, say so clearly.
"""
tags = ["review", "correctness", "codex"]

[runtime]
context_mode = "SummaryOnly"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "ReadOnly"
approval = "DenyByDefault"
timeout_secs = 900
spawn_policy = "Sync"

[provider_overrides.codex]
model_reasoning_effort = "high"

[workflow]
enabled = true
stages = ["Review"]
max_runtime_depth = 1
```

### 14.5 `style-reviewer.agent.toml`

```toml
[core]
name = "style-reviewer"
description = "Reviews maintainability, naming, readability, and team consistency."
provider = "Claude"
model = "sonnet"
instructions = """
You are a maintainability and style reviewer.
Focus on readability, naming, structure, documentation, and long-term maintainability.
Do not claim correctness proof unless evidence exists.
"""
tags = ["review", "style", "claude", "maintainability"]

[runtime]
context_mode = "SummaryOnly"
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "ReadOnly"
approval = "DenyByDefault"
timeout_secs = 900
spawn_policy = "Sync"

[workflow]
enabled = true
stages = ["Review", "Archive"]
max_runtime_depth = 1
```

### 14.6 `local-fallback-coder.agent.toml`

```toml
[core]
name = "local-fallback-coder"
description = "Optional local fallback coding agent backed by Ollama for non-critical or offline tasks."
provider = "Ollama"
model = "qwen2.5-coder"
instructions = """
You are a local fallback coding agent.
Prefer small, contained changes.
If confidence is low, return open_questions instead of pretending certainty.
"""
tags = ["build", "local", "ollama", "fallback"]

[runtime]
context_mode = { SelectedFiles = ["src/**", "PLAN.md"] }
memory_sources = ["AutoProjectMemory", "ActivePlan"]
working_dir_policy = "Auto"
file_conflict_policy = "Serialize"
sandbox = "WorkspaceWrite"
approval = "DenyByDefault"
timeout_secs = 1200
spawn_policy = "Async"

[workflow]
enabled = true
stages = ["Build"]
max_runtime_depth = 1
```

---

## 15. 建议新增的 `init` 预设（v0.7 推荐实现）

建议在下一阶段新增：

```bash
mcp-subagent init --preset claude-opus-supervisor
```

它应自动生成：

- `agents/` 默认团队
- `PLAN.md` 模板
- `.mcp-subagent/config.toml`
- `README.mcp-subagent.md`
- 接入 Claude / Codex / Gemini 的一键命令提示

### 15.1 预设种类建议

- `claude-opus-supervisor`（你的默认场景）
- `codex-primary-builder`
- `gemini-frontend-team`
- `local-ollama-fallback`
- `minimal-single-provider`

---

## 16. 对“节省 Token / 提高效率”的最终建议

### 16.1 最有效的不是换更多模型，而是减少主线程污染

真正最省 token 的方式是：

- 主线程只保留关键决策
- 把探索、扫描、尝试、失败日志都留在子代理上下文里
- 子代理只带回来结构化结果

### 16.2 推荐你的默认做法

#### 默认

- Claude Code 主会话：`opusplan`
- 研究：`fast-researcher`
- 后端编码：`backend-coder`
- 前端编码：`frontend-builder`
- correctness review：`correctness-reviewer`
- style review：`style-reviewer`

#### 高风险架构决策

- Claude Code 主会话切到 `opus`
- Build 仍交给 Codex / Gemini
- Review 必须双审

#### 轻量任务

- Claude Code 主会话直接 `sonnet`
- 不必每次都进完整 workflow

### 16.3 一个实用策略

- **重脑子工作**：交给主管模型  
- **重体力工作**：交给子代理  
- **重验证工作**：交给 reviewer  
- **重沉淀工作**：交给 Archive / Knowledge Capture

---

## 17. 给实现同事的最终开发顺序（拍板）

### 17.1 第一优先级

1. 修 Gemini / Claude provider flag 映射
2. 更新 README / docs / examples 到 `mcp-subagent mcp`
3. 文档明确 stdio-only
4. 为“不能保证零幻觉，只能提升可核验性”补一节

### 17.2 第二优先级

1. 增加 `init --preset`
2. 增加 `--selected-file-inline`
3. 完成 workflow gate 其余条件
4. 增加自动归档与 knowledge capture hook

### 17.3 第三优先级

1. stage-aware routing
2. finer-grained conflict lock
3. richer doctor / compatibility report
4. 预设团队和样例仓库

---

## 18. 最终拍板结论

这就是我给你的最终结论：

1. **当前仓库值得继续做，方向正确。**  
2. **你的实际场景应该默认采用：Claude 主管 + Codex/Gemini 子代理。**  
3. **主代理负责方向，子代理负责执行，reviewer 负责纠偏，archive 负责记忆。**  
4. **plan / summary / artifacts 是公共控制面，远比“让更多 agent 自由发挥”更重要。**  
5. **下一版不要继续发散加功能，而是优先把 provider 映射、文档、预设、接入体验做扎实。**

如果按这份 v0.7 去实现，`mcp-subagent-rs` 会从“多 provider CLI runtime”升级成一个真正可长期使用的 **本地多 LLM 工作流 runtime**。

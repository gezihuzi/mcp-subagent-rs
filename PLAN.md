# PLAN.md

## North Star

按 `docs/mcp-subagent_tech_design_v0.8.md` 交付首个“直接可用 beta”：首次接入路径可复制可运行、命令面与文档零漂移、默认场景稳定可验证。

## Execution Strategy (v0.8 Current)

### Batch V0.8-P0 - First Success Path（当前优先）

目标：完成 `connect-snippet + init README + smoke_v08/CI + release docs/changelog/version + real examples/onboarding + CI reliability fixes` 收口，让用户首次接入、发布切点和示例落地都可复制可验证。
依赖顺序：`T-059 -> T-060 -> T-061 -> T-062 -> T-064 -> T-065 -> T-066 -> T-067 -> T-068`。
回滚策略：新增命令面与模板升级均保持向后兼容，不影响既有 `mcp/doctor/validate/run/spawn` 主链。
风险与控制：路径绝对化与 shell 转义实现不当会导致复制失败；smoke 误依赖本机真实 codex 会导致 CI 不稳定。通过单测覆盖 host 模板、绝对路径和含空格路径转义，并在 smoke 中使用 fake codex runner 固定回归路径。

## Execution Strategy (Module Batches)

### Batch A - Runtime 可直接操作（当前优先）

目标：先补齐本地 CLI 命令面与 summary envelope 主路径，让开发与调试不依赖 MCP Host，且结构化输出可稳定吸收。
依赖顺序：`T-029 -> T-030`。
回滚策略：CLI 与 summary 改造均保持向后兼容读取，不破坏现有 MCP tools 协议字段。

### Batch B - Workflow 一等能力

目标：引入 `WorkflowSpec`、`ActivePlan`、stage-aware dispatcher、plan gate 与深度限制，让协作从 prompt 驱动转为工件驱动。
依赖顺序：`T-031 -> T-032 -> T-033`。
风险：schema 扩展与运行时 gate 同步推进时可能产生兼容窗口。
回滚策略：workflow 字段保持可选，gate 默认保守，不影响旧 spec 的基础执行路径。

### Batch C - 策略与 provider 分层

目标：实现 `WorkingDirPolicy::Auto`，并把 Mock/Ollama 层级语义收口到 v0.6 定义。
依赖顺序：`T-034 -> T-035 -> T-036`。
风险：provider 语义切换会影响现有测试基线（当前使用 Ollama 走 mock）。
回滚策略：先引入 Mock 并保留旧行为兼容窗口，再逐步移除 Ollama-mock 假设。

### Batch D - 可观测性与状态布局

目标：对齐 run 目录布局与 events 事件流，增强重启恢复和审计能力。
依赖顺序：`T-037`。
风险：持久化结构变更造成历史数据读取回归。
回滚策略：版本化读取和 fallback 兼容旧 `run.json`。

### Batch E - MVP 验收收口

目标：完成 v0.6 MVP smoke、文档与状态声明，形成可重复验收标准。
依赖顺序：`T-038`。
回滚策略：文档和脚本独立提交，不影响 runtime 主逻辑。

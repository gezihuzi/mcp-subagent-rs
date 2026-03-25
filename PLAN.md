# PLAN.md

## North Star

按 `docs/mcp-subagent_tech_design_v0.10.md` 推进“可观察、可解释、可中断”的本地多 LLM 子代理 runtime：`spawn` 先接受、执行后置，任务阶段与阻塞原因可见，结果 native-first、统计可追踪。

## Execution Strategy (v0.10 Current)

### Batch V0.10-P0 - Spawn Accepted-only + Runtime Transparency（当前优先）

目标：先消除“spawn 黑盒卡住”与“运行中不可观察”两类核心体验问题。首个切片先完成 `spawn` accepted-only 语义（同步路径不做 provider probe），随后补事件流/心跳/watch/stats，并补 `block_reason + logs --follow + waiting/watchdog events + stats phase splits + phase_progress view + phase filter/timeout + MCP phase watchdog + watch advice` 让阻塞可解释。
依赖顺序：`T-086(Completed) -> T-087(Completed) -> T-088(Completed) -> T-089(Completed) -> T-090(Completed) -> T-091(Completed) -> T-092(Completed) -> T-093(Completed) -> T-094(Completed) -> T-095(Completed) -> T-096(Completed) -> T-097(Completed) -> T-098(Completed) -> T-099(Completed) -> T-100(Completed) -> T-101(Completed) -> T-102(Completed) -> T-103(Completed) -> T-104(Completed) -> T-105(Completed) -> T-106(Completed) -> T-107(Completed) -> T-108(Completed) -> T-109(Completed) -> T-110(Completed) -> T-111(Completed)`。
回滚策略：`run_agent` 保持同步 probe 语义；`spawn_agent` 仅将 probe 后移到 worker，异常仍落盘同一 run 结构，必要时可回退到旧 `prepare_run + upfront probe` 路径。
风险与控制：provider 不可用从“同步拒绝”变成“异步失败”可能影响调用侧预期；通过保留明确 `error_message`（含 unavailable 原因）和测试覆盖（slow probe 快返、unavailable 异步失败）降低误解。

## Execution Strategy (v0.9 Current)

### Batch V0.9-P0 - Delegation Minimal + Native-first（当前优先）

目标：先完成默认策略收口和失败语义修正：`memory_sources` 默认去掉 `ActivePlan`、新增 `delegation_context/native_discovery/output_mode/parse_policy`、`parse_policy=best_effort` 下 provider 成功不因归一化失败判 hard fail、补 `submit` 命令别名。
依赖顺序：`T-072 -> T-073 -> T-074`。
回滚策略：新策略字段全部有默认值，旧 agent spec 可无缝加载；`spawn/status` 兼容保留，`submit` 只是别名扩展。
风险与控制：放宽解析可能掩盖格式问题；通过在 summary 中保留 `parse_status` 与 raw artifact，并在 strict 模式保留旧失败语义。

### Batch V0.9-P1 - MCP Run Result Surface（已完成）

目标：在 MCP 工具面补齐 run 可观测能力：`list_runs/get_run_result/read_run_logs/watch_run`，让 host 不需要拼 `status + artifact` 才能消费结果。
依赖顺序：`T-075 -> T-076 -> T-077 -> T-078 -> T-079 -> T-080 -> T-081`（已完成 native usage 采集与结果面回填）。
回滚策略：新增 MCP tools 仅扩展协议面，不破坏既有 `list_agents/run_agent/spawn_agent/get_agent_status/cancel_agent/read_agent_artifact`。
风险与控制：watch 轮询可能带来频繁 IO；通过最小轮询间隔（50ms）与可配置 timeout 控制开销。

### Batch V0.9-P2 - Run Timeline + Usage + Retry Observability（已完成 T-082/T-083/T-084/T-085）

目标：在已完成 `timeline`、usage 精度和 ambient 诊断基础上，补齐 retry 分类可观测性（仅输出，不变更重试策略）。
依赖顺序：`T-082 -> T-083 -> T-084 -> T-085`（Completed 2026-03-25：输出层 retry classification 已落地，执行策略未改动）。
回滚策略：仅新增输出字段与事件，不改变执行重试分支；移除字段即可无损回滚。
风险与控制：错误文案规则可能误分类；通过保守 `unknown` 分类与 reason 明示降低误导。

## Execution Strategy (v0.8 Current)

### Batch V0.8-P0 - First Success Path（当前优先）

目标：完成 `connect-snippet + init README + smoke_v08/CI + release docs/changelog/version + real examples/onboarding + CI reliability fixes + summary parsing robustness` 收口，让用户首次接入、发布切点和示例落地都可复制可验证。
依赖顺序：`T-059 -> T-060 -> T-061 -> T-062 -> T-064 -> T-065 -> T-066 -> T-067 -> T-068 -> T-069 -> T-070 -> T-071`。
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

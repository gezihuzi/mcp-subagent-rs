# TODO.md

## T-001 Phase0-CoreSpecContextSummary (Completed 2026-03-24)
任务：实现 AgentSpec 三层结构、校验器、ContextCompiler 固定模板、Summary 哨兵 JSON 解析与降级。
验收标准：
1. `*.agent.toml` 可被加载到 `AgentSpec`，未知字段会报错。
2. 非当前 provider 的 override 会被校验器拒绝。
3. 编译结果包含固定段落：ROLE/TASK/OBJECTIVE/CONSTRAINTS/ACCEPTANCE CRITERIA/SELECTED CONTEXT/RESPONSE CONTRACT/OUTPUT SENTINELS。
4. 能从 `<<<MCP_SUBAGENT_SUMMARY_JSON_START>>>...<<<MCP_SUBAGENT_SUMMARY_JSON_END>>>` 解析 `StructuredSummary`。
5. 缺失或非法 JSON 时返回 `verification_status = ParseFailed` 的降级摘要。
6. `cargo test` 通过且覆盖上述路径。
完成记录：
- 已实现 `spec` 模块（core/runtime_policy/provider_overrides/validate/registry）。
- 已实现 `runtime::context` 固定模板编译，包含 8 个必备段落。
- 已实现 `runtime::summary` 哨兵 JSON 解析与 ParseFailed 降级。
- 已新增 9 个单元测试并通过 `cargo test`。

## T-002 Phase1-RuntimeStateMock (Completed 2026-03-24)
任务：实现 dispatcher 生命周期状态机与 mock runner。
验收标准：run 流程可从 RECEIVED 走到 SUCCEEDED/FAILED/TIMED_OUT/CANCELLED，产出 run 元数据和 summary。
完成记录：
- 已实现 `runtime::dispatcher`，覆盖完整生命周期状态流与 run metadata。
- 已实现 `runtime::mock_runner`，可模拟成功/失败/超时/取消四类终态。
- Dispatcher 在四类终态均返回 summary（失败类会降级为 ParseFailed）。
- 已新增状态机与 mock runner 测试并通过 `cargo test`。

## T-003 Phase2-MCPStdioListRun (Completed 2026-03-24)
任务：接入 rmcp stdio 最小 server，暴露 list_agents/run_agent。
验收标准：`mcp-subagent --mcp` 可启动并响应基础工具调用。
完成记录：
- 已接入 `rmcp = 1.2.0`（`server/macros/transport-io`）并实现 `McpSubagentServer`。
- 已实现 `--mcp` 启动入口（stdio transport）。
- 已实现并暴露 `list_agents`、`run_agent` 两个 MCP tool。
- 已新增 `mcp::server` 单测覆盖 `list_agents/run_agent` 返回结构。
- `cargo test` 通过（17 passed）。
残余风险：
- 已覆盖 rmcp duplex 端到端调用；仍未覆盖 Claude Desktop/Cursor 等宿主集成测试。

## T-004 Phase2-MCPAsyncStatusArtifact (Completed 2026-03-24)
任务：补齐 MCP 异步运行工具与状态/产物读取能力。
验收标准：
1. MCP 暴露 `spawn_agent/get_agent_status/cancel_agent/read_agent_artifact` 四个工具。
2. `spawn_agent` 立即返回 `handle_id`，随后可通过 `get_agent_status` 看到状态推进到终态。
3. `cancel_agent` 对运行中任务生效，并将状态标记为 `Cancelled`。
4. `read_agent_artifact` 可读取 UTF-8 文本 artifact（至少覆盖 `summary.json`）。
5. `cargo test` 通过，并新增 duplex 协议测试覆盖新工具链路。
完成记录：
- 已在 MCP server 暴露 4 个新增工具，并保留 `list_agents/run_agent`。
- 已实现内存态 run registry（运行状态、错误信息、summary、artifact 索引与文本内容）。
- `spawn_agent` 立即返回 `handle_id`，后台异步执行后可由 `get_agent_status` 轮询终态。
- `cancel_agent` 可中止运行中任务并写入 `Cancelled` 状态与取消摘要。
- `read_agent_artifact` 已可读取 `summary.json/stdout.txt/stderr.txt` 等文本 artifact。
- 已新增 duplex 端到端测试覆盖上述工具链路，`cargo test` 通过（17 passed）。

## T-005 Phase3-StatePersistence (Completed 2026-03-24)
任务：将 run 状态和文本 artifact 落盘，支持服务重启后继续查询。
验收标准：
1. 每次 run 的状态与摘要持久化到 state 目录（按 handle_id 分目录）。
2. `summary.json/stdout.txt/stderr.txt` 等文本 artifact 持久化到磁盘。
3. 服务进程重启后，`get_agent_status` 与 `read_agent_artifact` 仍可读取历史 run。
4. 对非法 artifact 路径（绝对路径/目录穿越）做拒绝处理。
5. `cargo test` 通过，并新增覆盖“重启后查询历史 run”场景。
完成记录：
- 已新增 state 目录持久化：`state/runs/<handle_id>/run.json` 与 `state/runs/<handle_id>/artifacts/*`。
- `run_agent/spawn_agent/cancel_agent` 均会更新内存态并落盘；异步任务完成后自动持久化终态。
- `get_agent_status/read_agent_artifact` 支持按 handle_id 从磁盘懒加载历史 run，实现重启后可查。
- 已实现 artifact 路径安全校验，拒绝绝对路径与目录穿越。
- 已新增测试 `restart_can_query_persisted_runs_and_reject_invalid_path`，全量 `cargo test` 通过（18 passed）。

## T-006 Phase2-ProviderProbeAvailability (Completed 2026-03-24)
任务：实现最小 provider probe，并将可用性接入 MCP `list_agents` 与运行前校验。
验收标准：
1. 新增 provider probe 抽象，至少覆盖 `Ready/MissingBinary/ProbeFailed/ExperimentalUnavailable` 状态。
2. `list_agents` 返回每个 agent 的真实 `available` 与 `capability_notes`（含 probe 状态说明）。
3. `run_agent/spawn_agent` 在 provider 不可用时直接拒绝启动并返回清晰错误。
4. 新增单测覆盖“provider 可用可运行”和“provider 不可用被拒绝”路径。
5. `cargo test` 全量通过。
完成记录：
- 已新增 `probe` 模块：`ProviderProber` 抽象、`SystemProviderProber` 实现、`ProviderProbe/ProbeStatus` 结构与状态语义。
- `McpSubagentServer` 已接入 provider probe：`list_agents` 根据探测结果返回 `available` 与 `capability_notes`。
- `run_agent/spawn_agent` 已新增 provider 可用性前置校验，不可用时返回明确错误。
- 已新增测试 `list_agents_marks_provider_unavailable` 与 `run_agent_rejects_unavailable_provider`，并将现有 MCP 测试切到可控 probe。
- 已通过 `cargo fmt && cargo test`（20 passed）与 `cargo run -- validate`。

## T-007 Phase1-CodexRunnerMVP (Completed 2026-03-24)
任务：接入最小可用 CodexRunner，使 Codex provider 可走真实 CLI 执行链路。
验收标准：
1. 新增 `CodexRunner`，使用 `codex exec` 非交互执行并支持 `timeout_secs`。
2. Codex provider 在 `run_agent/spawn_agent` 中走真实 runner，非 Codex provider 暂保留 mock runner。
3. 运行结束后仍能产出并持久化 `summary.json/stdout.txt/stderr.txt`。
4. 新增 runner 级单测（fake codex binary）覆盖成功与失败/超时至少两条路径。
5. `cargo test` 全量通过。
完成记录：
- 已新增 `runtime::codex_runner`：基于 `codex exec` 非交互执行，支持 sandbox/model/reasoning 配置透传和超时处理。
- MCP server 分发已改为：`Provider::Codex` 走真实 CodexRunner，其他 provider 继续走 mock dispatcher，避免一次性引入多 provider 风险。
- Codex 路径与既有状态持久化/artifact 逻辑已打通，仍会落盘 `summary.json/stdout.txt/stderr.txt`。
- 已新增 `runtime::codex_runner` 单测：fake codex binary 成功输出路径、非零退出失败路径。
- 已通过 `cargo test`（22 passed）与 `cargo run -- validate`。

## T-008 Phase1-GeminiRunnerMVP (Completed 2026-03-24)
任务：接入最小可用 GeminiRunner，使 Gemini provider 可走真实 CLI 执行链路。
验收标准：
1. 新增 `GeminiRunner`，使用 `gemini --prompt` 非交互执行并支持 `timeout_secs`。
2. Gemini provider 在 `run_agent/spawn_agent` 中走真实 runner，其他未接入 provider 暂保留 mock runner。
3. 运行结束后仍能产出并持久化 `summary.json/stdout.txt/stderr.txt`。
4. 新增 runner 级单测（fake gemini binary）覆盖成功与失败/超时至少两条路径。
5. `cargo test` 全量通过。
完成记录：
- 已新增 `runtime::gemini_runner`：基于 `gemini --prompt` 非交互执行，支持 model/approval-mode 映射与超时处理。
- MCP server 分发已改为：`Provider::Gemini` 走真实 GeminiRunner，`Provider::Codex` 走 CodexRunner，其他 provider 仍走 mock。
- Gemini 路径与既有 summary 解析、state 持久化、artifact 输出逻辑已打通，仍会产出 `summary.json/stdout.txt/stderr.txt`。
- 已新增 `runtime::gemini_runner` 单测：fake binary 成功输出路径、非零退出失败路径、超时路径。
- 已通过 `cargo test`（25 passed）与 `cargo run -- validate`。

## T-009 Phase2-ClaudeRunnerMVP (Completed 2026-03-24)
任务：接入最小可用 ClaudeRunner，使 Claude provider 可走真实 CLI 执行链路。
验收标准：
1. 新增 `ClaudeRunner`，使用 `claude --print` 非交互执行并支持 `timeout_secs`。
2. Claude provider 在 `run_agent/spawn_agent` 中走真实 runner，其他未接入 provider 暂保留 mock runner。
3. 运行结束后仍能产出并持久化 `summary.json/stdout.txt/stderr.txt`。
4. 新增 runner 级单测（fake claude binary）覆盖成功与失败/超时至少两条路径。
5. `cargo test` 全量通过。
完成记录：
- 已新增 `runtime::claude_runner`：基于 `claude --print` 非交互执行，支持 model/permission-mode 映射与超时处理。
- MCP server 分发已改为：`Provider::Claude` 走真实 ClaudeRunner，`Provider::Codex` 走 CodexRunner，`Provider::Gemini` 走 GeminiRunner，其他 provider 仍走 mock。
- Claude 路径与既有 summary 解析、state 持久化、artifact 输出逻辑已打通，仍会产出 `summary.json/stdout.txt/stderr.txt`。
- 已新增 `runtime::claude_runner` 单测：fake binary 成功输出路径、非零退出失败路径、超时路径。
- 为避免宿主登录态影响 MCP 单测，测试默认 provider 已切到 `Ollama`（走 mock 路径）。
- 已通过 `cargo test`（28 passed）与 `cargo run -- validate`。

## T-010 Phase2-DoctorCommandProbeReport (Completed 2026-03-24)
任务：新增 `doctor` 诊断子命令，输出本地 provider 探测和运行目录信息。
验收标准：
1. CLI 新增 `mcp-subagent doctor [agents_dir]` 子命令。
2. `doctor` 输出包含：cwd、agents_dirs、state_dir、agent specs 统计信息。
3. `doctor` 输出包含 Claude/Codex/Gemini/Ollama 的 probe 结果（status/version/bin/notes）。
4. 新增单测覆盖 doctor 报告构建与文本渲染的关键字段。
5. `cargo test` 全量通过。
完成记录：
- 已新增 `doctor` 模块（report 构建 + 文本渲染），输出 cwd、agents/state 路径、spec 统计与 provider probe 详情。
- CLI 已新增 `mcp-subagent doctor [agents_dir]` 子命令，并更新 usage。
- `doctor` 使用现有 `ProviderProber` 能力，按 Claude/Codex/Gemini/Ollama 顺序输出状态、版本、可执行文件与 notes。
- 已新增单测 `doctor::tests::builds_report_and_renders_key_fields` 覆盖报告构建与渲染关键字段。
- 已通过 `cargo fmt && cargo test`（29 passed），并验证 `cargo run -- doctor` 与 `cargo run -- validate`。

## T-011 Phase2-UnifiedConfigAndClapCLI (Completed 2026-03-24)
任务：将 `agents_dir/state_dir` 提升为统一配置来源，并迁移 CLI 参数解析到 `clap`。
验收标准：
1. 新增统一配置解析层，支持默认值、配置文件、环境变量、CLI 参数覆盖。
2. `mcp/doctor/validate` 三个命令都复用同一份解析后的 runtime config。
3. CLI 从手动 `env::args` 迁移为 `clap` 子命令模式，参数定义可扩展。
4. 新增配置合并逻辑单测，覆盖优先级（CLI > ENV > File > Default）。
5. `cargo test` 全量通过。
完成记录：
- 已新增 `config` 模块，统一解析 `agents_dirs/state_dir`，支持默认值、`config.toml`、`MCP_SUBAGENT_*` 环境变量和 CLI 覆盖。
- 已将 `src/main.rs` 迁移到 `clap`：`mcp-subagent mcp|doctor|validate` 子命令模式，并复用统一 config 解析。
- `run_mcp_server/doctor/validate` 已全部改为接收统一 `RuntimeConfig`。
- 已新增配置优先级单测 `config::tests::merge_*`。
- 已通过 `cargo fmt && cargo test`（36 passed），并验证 `cargo run -- doctor` 与 `cargo run -- validate`。

## T-012 Phase2-ProbeStatusRefinement (Completed 2026-03-24)
任务：增强 provider probe 状态分类，区分权限受限失败与通用探测失败。
验收标准：
1. `ProbeStatus` 新增权限受限状态，避免将权限问题混为 `ProbeFailed`。
2. probe 推断逻辑能区分 `PermissionDenied/NeedsAuthentication/ExperimentalUnavailable/UnsupportedVersion`。
3. 对“命令成功但含非致命告警”场景做保守处理，避免误判不可用。
4. 新增状态推断单测覆盖关键分类路径。
5. `cargo test` 全量通过。
完成记录：
- 已为 `ProbeStatus` 新增 `PermissionDenied`，并更新系统 probe 错误映射逻辑。
- 已实现文本规则推断函数，细分权限、认证、实验特性、版本不支持等失败原因。
- 已补充“成功 + 非致命权限告警”回退规则：若检测到有效版本行则保持 `Ready` 并追加说明 note。
- 已新增 `probe::tests::*` 单测覆盖权限/认证/实验特性/成功版本行等路径。
- 已通过 `cargo fmt && cargo test`（36 passed），并验证 `doctor` 输出状态分类符合预期。

## T-013 Phase3-WorkspacePolicyAndConflictControl (Completed 2026-03-24)
任务：落地 `working_dir_policy` 与 `file_conflict_policy=Serialize` 的运行时执行逻辑，并把 workspace 元信息写入 run 状态。
验收标准：
1. 新增 workspace manager，支持 `InPlace/TempCopy/GitWorktree`（GitWorktree 失败时回退 TempCopy 并记录原因）。
2. `run_agent/spawn_agent` 在执行前按 policy 准备 workspace，并将实际 workspace 路径用于 runner 执行。
3. 对 `file_conflict_policy=Serialize` + 写权限任务启用同仓库串行锁，避免并发写冲突。
4. `run.json` 增加 workspace 元信息（mode/source/workspace/notes/lock_key）。
5. 新增测试覆盖 workspace 策略、run metadata 持久化、串行锁阻塞行为。
6. `cargo test` 全量通过。
完成记录：
- 已新增 `runtime::workspace` 模块，实现 `prepare_workspace` 与 `resolve_source_path`，覆盖 `InPlace/TempCopy/GitWorktreeFallback`。
- MCP server 的 `run_dispatch` 已改为先准备 workspace，再按 provider 执行；执行请求的 `working_dir` 使用实际 workspace 路径。
- 已新增 serialize lock（按源仓库路径 key）并接入 `run_agent/spawn_agent`，对写任务生效。
- `PersistedRunRecord` 已新增 `workspace` 字段并在 `run.json` 持久化（mode/source/workspace/notes/lock_key），重启后可加载。
- 已新增测试：
  - `runtime::workspace::*`（策略行为）
  - `mcp::server::run_agent_tempcopy_persists_workspace_metadata`
  - `mcp::server::serialize_lock_blocks_until_guard_released`
- 已通过 `cargo fmt && cargo test`（41 passed），并验证 `cargo run -- doctor` 与 `cargo run -- validate`。

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
验收标准：`mcp-subagent mcp` 可启动并响应基础工具调用。
完成记录：

- 已接入 `rmcp = 1.2.0`（`server/macros/transport-io`）并实现 `McpSubagentServer`。
- 已实现 `mcp` 子命令启动入口（stdio transport）。
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

## T-014 Phase2-ValidateMemoryPathAndSummaryContract (Completed 2026-03-24)

任务：增强 `validate` 子命令覆盖设计文档要求的 memory/source 路径与 summary contract 模板完整性校验。
验收标准：

1. `validate` 能拒绝非法 `memory_sources` 路径（空值、绝对路径、`..` 目录穿越、`File(...)` 误用 glob）。
2. `validate` 会校验 ContextCompiler 模板仍包含 8 个固定段落与 summary 哨兵。
3. `load_agent_specs_from_dirs` 路径上的 spec 校验自动包含上述 memory/source 规则。
4. 新增单测覆盖 memory/source 校验失败与模板校验失败路径。
5. `cargo test` 全量通过。
完成记录：

- 已扩展 `spec::validate`：新增 `memory_sources` 校验逻辑，覆盖 `AutoProjectMemory/File/Glob/Inline` 的路径与内容规则。
- 已新增 `runtime::context::validate_default_summary_contract_template` 与 `validate_compiled_prompt_template`，确保模板完整性与哨兵约束。
- `validate` 子命令已接入模板校验；若模板破坏会直接失败返回。
- 已新增单测：
  - `spec::validate::*`（absolute/parent traversal/glob misuse/empty inline/valid paths）
  - `runtime::context::validates_default_summary_contract_template`
  - `runtime::context::rejects_template_missing_required_sections`

## T-015 Phase3-ResolvedMemoryAndDedup (Completed 2026-03-24)

任务：实现真实的 `ResolvedMemory` 解析链路，并覆盖 provider 原生记忆文件不重复内联的去重要求。
验收标准：

1. `run_agent/spawn_agent` 不再使用 `ResolvedMemory::default()`，改为按 `runtime.memory_sources` 动态解析。
2. `AutoProjectMemory` 至少解析 `PROJECT.md` 与 provider 原生记忆文件路径（native passthrough）。
3. `File/Glob/Inline` memory source 能被解析并注入 `ResolvedMemory`。
4. 去重策略覆盖“provider 原生文件不重复内联”场景（显式 `File` 命中 native 文件时移除 native passthrough）。
5. 新增单测覆盖 auto memory、glob 解析、native 去重与空 glob 失败路径。
6. `cargo test` 全量通过。
完成记录：

- 已新增 `runtime::memory` 模块：`resolve_memory` 支持 `AutoProjectMemory/File/Glob/Inline`。
- `AutoProjectMemory` 已支持：
  - 项目记忆候选：`PROJECT.md`、`.mcp-subagent/PROJECT.md`
  - provider native 记忆候选：Claude(`CLAUDE.md/.claude/CLAUDE.md`)、Codex(`AGENTS.md/AGENTS.override.md`)、Gemini(`GEMINI.md`)
- 已实现 memory 内容读取与截断保护（32KB 上限，保留截断标记）。
- 已接入 `mcp::server::run_dispatch`，真实 runner 与 mock runner 均使用解析后的 memory。
- 已新增测试：
  - `runtime::memory::auto_project_memory_resolves_project_and_native_paths`
  - `runtime::memory::explicit_file_memory_dedups_native_passthrough`
  - `runtime::memory::glob_memory_source_inlines_all_matches`
  - `runtime::memory::glob_memory_source_requires_at_least_one_match`
- 已通过 `cargo fmt && cargo test`（52 passed）与 `cargo run -- validate`。

## T-016 Phase3-RunJsonSnapshotsAndRunLogs (Completed 2026-03-24)

任务：按技术设计补齐 run 状态持久化内容，落地 request/spec/probe 快照与 run 级日志文件。
验收标准：

1. `run.json` 增加 request snapshot、spec snapshot、probe result、created_at、status_history。
2. `run.json` 对旧版本数据保持兼容读取（新增字段缺失时可加载）。
3. 每个 run 目录固定落盘 `stdout.log`、`stderr.log` 与 `temp/` 目录。
4. 运行中/取消/失败路径都会维护 `status_history` 终态。
5. 新增测试覆盖 run metadata 扩展字段与日志文件落盘。
6. `cargo test` 全量通过。
完成记录：

- 已扩展 `RunRecord/PersistedRunRecord`，新增 `created_at/status_history/request_snapshot/spec_snapshot/probe_result`。
- `prepare_run` 现在返回探测结果，`run_agent/spawn_agent` 在持久化前写入 probe 快照。
- 已新增快照构建函数（request/spec/probe），并为取消/失败路径补齐状态历史更新。
- `persist_run_record` 已固定写入 `<run>/stdout.log`、`<run>/stderr.log`，并确保 `<run>/temp/` 存在。
- 已增强测试 `run_agent_tempcopy_persists_workspace_metadata`，校验新字段和日志文件。
- 已通过 `cargo fmt && cargo test`（52 passed）与 `cargo run -- validate`。

## T-017 Phase3-ArtifactPolicyWorkspaceMaterialization (Completed 2026-03-24)

任务：补齐 artifact policy，使 `summary.artifacts` 中声明的文本产物可被读取与持久化，而不是只在索引中展示。
验收标准：

1. `build_runtime_artifacts` 能从 workspace 解析并读取 `summary.artifacts` 声明的文本文件内容。
2. 仅允许 workspace 内部路径，拒绝目录穿越或越界路径。
3. `run_agent/spawn_agent` 成功路径都接入 artifact materialization。
4. 新增测试覆盖“声明 artifact 被落盘并可读”路径。
5. `cargo test` 全量通过。
完成记录：

- 已扩展 `build_runtime_artifacts(summary, stdout, stderr, workspace_root)`，支持从 workspace 采集 `summary.artifacts` 文本内容并写入 artifact payload。
- 已新增 `resolve_artifact_path_in_workspace`，通过 canonical path 限制 artifact 只读 workspace 内文件。
- `run_agent` 与 `spawn_agent` 成功路径已传入实际 workspace 根路径。
- 已新增测试 `declared_workspace_artifacts_are_persisted_in_index_and_payloads`。
- 已通过 `cargo fmt && cargo test`（53 passed）与 `cargo run -- validate`。

## T-018 Phase3-TracingAndLogLevelSurface (Completed 2026-03-24)

任务：补齐日志基线能力，支持 `RUST_LOG` / `--log-level`，并把 server 全局日志落到 state 目录。
验收标准：

1. CLI 支持全局 `--log-level` 参数，且优先级高于 `RUST_LOG` 与配置默认值。
2. runtime config 支持 `server.log_level`（配置文件）并保持 `CLI > ENV > File > Default` 合并顺序。
3. 进程启动后初始化 tracing subscriber，并把日志写到 `stderr + <state_dir>/server.log`。
4. 新增单测覆盖日志级别解析优先级与配置合并逻辑。
5. `cargo test` 全量通过。
完成记录：

- 已新增 `logging` 模块，落地 tracing 初始化与日志级别解析。
- CLI 已新增全局 `--log-level`，并在 `mcp/doctor/validate` 路径统一初始化 logging。
- `config` 已扩展 `RuntimeConfig.log_level` 与 `[server].log_level` 解析，支持 `MCP_SUBAGENT_LOG_LEVEL`。
- 已新增测试：
  - `logging::tests::*`（CLI/env/config 优先级）
  - `config::tests::merge_*`（含 log level 断言）
- 已通过 `cargo fmt && cargo test`（56 passed）与 `cargo run -- validate`。

## T-019 V0.6-P0-1-ContextModeBehavior (Completed 2026-03-24)

任务：按 v0.6 文档落地 P0-1，让 `context_mode` 真正控制上下文注入分支。
验收标准：

1. `Isolated/SummaryOnly/SelectedFiles/ExpandedBrief` 四种模式有真实注入差异，不再“统一全带”。
2. `SelectedFiles` 仅注入 allowlist 文件；`SummaryOnly` 不注入 selected files 正文；`Isolated` 不注入 parent summary。
3. `ExpandedBrief` 注入 parent summary digest，而非原文全文。
4. 具备“raw transcript 风险抑制”校验，出现明显对话转录格式时不直接注入。
5. 单测覆盖四种模式与 raw transcript 抑制路径。
6. `cargo test` 全量通过。
完成记录：

- 已在 `runtime::context` 新增 `ContextInjectionPolicy`，按 `context_mode` 决定 parent summary / selected files 注入范围。
- 已实现 `SelectedFiles` allowlist 匹配、`ExpandedBrief` digest 生成、以及 raw transcript 形态抑制逻辑。
- 已新增测试：
  - `isolated_mode_excludes_parent_summary_and_selected_files`
  - `summary_only_mode_includes_parent_summary_but_excludes_selected_files`
  - `selected_files_mode_only_includes_allowlisted_files`
  - `expanded_brief_mode_uses_parent_summary_digest`
  - `summary_only_blocks_raw_transcript_like_parent_summary`
- 已通过 `cargo fmt && cargo test`（61 passed）与 `cargo run -- validate`。

## T-020 V0.6-P0-2a-McpServerDtoSplit (Completed 2026-03-24)

任务：推进 v0.6 P0-2 的第一步，先将 MCP 输入输出 DTO 从 `mcp/server.rs` 独立拆分，降低 server 文件职责密度并保持兼容。
验收标准：

1. 新增 `src/mcp/dto.rs`，集中承载 MCP tools 的输入/输出结构体。
2. `src/mcp/server.rs` 不再定义重复 DTO，改为引用独立模块。
3. 兼容旧调用路径（`mcp::server::*`）不破坏。
4. 功能与测试行为不回退。
5. `cargo test` 全量通过。
完成记录：

- 已新增 `src/mcp/dto.rs`，迁移 `RunAgentInput/Output`、`Spawn`、`Status`、`Artifact`、`ListAgents` 等 DTO。
- `src/mcp/mod.rs` 已注册 `pub mod dto;`。
- `src/mcp/server.rs` 已移除本地 DTO 定义并 `pub use crate::mcp::dto::*` 保持兼容导出。
- 已通过 `cargo fmt && cargo test`（61 passed）与 `cargo run -- validate`。

## T-021 V0.6-P0-2b-McpServerArtifactSplit (Completed 2026-03-24)

任务：推进 v0.6 P0-2 的第二步，将 artifact/path 相关逻辑从 `mcp/server.rs` 抽离到独立模块，继续降低 server 文件职责密度。
验收标准：

1. 新增/完善 `src/mcp/artifacts.rs`，集中承载 run 路径、artifact 路径净化、artifact 读取和 runtime artifact 构建逻辑。
2. `src/mcp/server.rs` 不再保留上述重复实现，统一调用 `mcp::artifacts`。
3. 现有工具行为不回退（`run_agent/spawn_agent/read_agent_artifact` 路径保持可用）。
4. 相关单测持续通过，特别是声明 artifact 持久化可读场景。
5. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增 `src/mcp/artifacts.rs`，落地：
  - `run_root_dir/run_dir/run_artifacts_dir`
  - `sanitize_relative_artifact_path`
  - `read_artifact_from_disk`
  - `build_runtime_artifacts` 及其 workspace artifact 解析辅助函数
- `src/mcp/mod.rs` 已注册 `pub(crate) mod artifacts;`。
- `src/mcp/server.rs` 已删除本地重复函数，改为统一引用 `mcp::artifacts`。
- 已通过 `cargo test`（61 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-022 V0.6-P0-2c-McpServerStateModelSplit (Completed 2026-03-24)

任务：推进 v0.6 P0-2 的第三步，将 MCP server 的状态模型与快照构建逻辑从 `mcp/server.rs` 抽离到独立模块，降低 server 结构耦合。
验收标准：

1. 新增 `src/mcp/state.rs`，承载 runtime state、run record、persisted record 与 snapshot 相关结构。
2. `src/mcp/server.rs` 不再定义重复状态模型与快照转换函数，改为复用 `mcp::state`。
3. `spawn/run/status/cancel` 行为不回退，持久化字段与加载逻辑保持兼容。
4. 现有 MCP 单测行为不变（尤其是重启读取历史 run 与串行锁场景）。
5. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增 `src/mcp/state.rs`，迁移：
  - `RuntimeState`、`RunRecord`、`WorkspaceRecord`、`PersistedRunRecord`
  - `RunRequestSnapshot`/`RunSpecSnapshot`/`ProbeResultRecord`
  - `build_run_*_snapshot`、`append_status_if_terminal`
- `src/mcp/mod.rs` 已注册 `pub(crate) mod state;`。
- `src/mcp/server.rs` 已改为引用 `mcp::state`，并移除本地重复定义与转换函数。
- 已通过 `cargo test`（61 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-023 V0.6-P0-2d-McpServerPersistenceSplit (Completed 2026-03-24)

任务：推进 v0.6 P0-2 的第四步，将 run 持久化读写职责从 `mcp/server.rs` 抽离到独立模块，继续收缩 server 文件职责。
验收标准：

1. 新增 `src/mcp/persistence.rs`，集中承载 run metadata/artifact 的持久化写入与重启后加载逻辑。
2. `src/mcp/server.rs` 不再定义 `persist/load` 相关函数，统一复用 `mcp::persistence`。
3. 持久化格式与兼容行为不回退（`run.json`、artifacts、stdout/stderr log、历史 run 加载）。
4. 现有覆盖重启查询/artifact 读取的 MCP 单测保持通过。
5. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增 `src/mcp/persistence.rs`，迁移：
  - `persist_run_record`
  - `load_run_record_from_disk`
  - `write_run_log_file`
  - 内部 `run_meta_path`
- `src/mcp/mod.rs` 已注册 `pub(crate) mod persistence;`。
- `src/mcp/server.rs` 已移除本地持久化函数，改为引用 `mcp::persistence::{persist_run_record, load_run_record_from_disk}`。
- 已通过 `cargo fmt && cargo test`（61 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-024 V0.6-P0-2e-McpServerToolEntrySplit (Completed 2026-03-24)

任务：推进 v0.6 P0-2 的第五步，将 MCP tool entry 从 `mcp/server.rs` 抽离到独立模块，进一步降低 server 文件职责密度。
验收标准：

1. 新增 `src/mcp/tools.rs`，承载 `list_agents/run_agent/spawn_agent/get_agent_status/cancel_agent/read_agent_artifact` 六个 tool 入口。
2. `src/mcp/server.rs` 不再包含 `#[tool_router]` 工具实现块，仅保留 server/service 骨架与运行链路。
3. 对外 MCP tool 名称与行为不回退，`new_with_state_dir_and_prober` 等 public API 保持兼容。
4. 相关可见性调整仅限 crate 内（`pub(crate)`），不扩大外部暴露面。
5. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增 `src/mcp/tools.rs`，迁移全部 6 个 MCP tool entry，并保留原有工具描述与返回结构。
- `src/mcp/server.rs` 已移除 `#[tool_router] impl`，改为通过 `mcp::tools::build_tool_router()` 装配 router。
- 为跨模块复用已将必要 helper（如 `run_dispatch`、summary/status 映射函数）收敛为 `pub(crate)`，未改变 crate 外 API。
- `src/mcp/mod.rs` 已注册 `pub(crate) mod tools;`。
- 已通过 `cargo fmt && cargo test`（61 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-025 V0.6-P0-2f-McpServerDispatchServiceSplit (Completed 2026-03-24)

任务：推进 v0.6 P0-2 的第六步，将 `run_dispatch` 与 provider 分发执行链从 `mcp/server.rs` 抽离到独立 service 模块。
验收标准：

1. 新增 `src/mcp/service.rs`，集中承载 `run_dispatch` 主链、workspace 记录映射、provider-specific dispatch 分支和 terminal metadata 组装。
2. `src/mcp/server.rs` 不再包含上述 dispatch 细节实现，职责收敛为 server/service 骨架与通用状态管理。
3. `src/mcp/tools.rs` 通过 service 模块调用 dispatch，不依赖 server 内部 dispatch 细节。
4. 行为不回退：`run_agent/spawn_agent` 在 Codex/Claude/Gemini/mock 路径保持一致。
5. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增 `src/mcp/service.rs`，迁移：
  - `DispatchEnvelope`
  - `run_dispatch`
  - `run_dispatch_{mock,codex,claude,gemini}`
  - `build_terminal_metadata`、workspace 映射与 mock summary helper
- `src/mcp/mod.rs` 已注册 `pub(crate) mod service;`。
- `src/mcp/server.rs` 已移除 dispatch 链实现并清理对应导入。
- `src/mcp/tools.rs` 已改为通过 `mcp::service::run_dispatch` 调用运行链路。
- 已通过 `cargo fmt && cargo test`（61 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-026 V0.6-P0-3-UnifiedAgentRunnerTrait (Completed 2026-03-24)

任务：按 v0.6 P0-3 统一真实 runner 抽象，让 mock 与真实 provider runner 走同一 trait 与同一 dispatcher 主链。
验收标准：

1. 新增统一 runner trait，mock/codex/claude/gemini 全部实现该 trait。
2. `Dispatcher` 仅依赖统一 trait，不再绑定 `mock_runner` 内部 trait。
3. `mcp/service.rs` 的 `run_dispatch()` 保留一条主链，不再存在 provider-specific 的执行分叉函数实现。
4. provider-specific 细节收敛在各 runner 模块内部（命令参数/超时/错误映射等）。
5. `cargo test` 与 `cargo run -- validate` 通过，行为不回退。
完成记录：

- 已新增 `src/runtime/runner.rs`，集中定义：
  - `AgentRunner`（async trait）
  - `RunnerExecution`
  - `RunnerTerminalState`
  - `Box<T>` 的 trait 转发实现
- `src/runtime/mock_runner.rs`、`codex_runner.rs`、`claude_runner.rs`、`gemini_runner.rs` 已统一实现 `AgentRunner`。
- `src/runtime/dispatcher.rs` 已改为异步 `run()`，统一走 `AgentRunner`；对应 dispatcher 测试改为 async 并保持通过。
- `src/mcp/service.rs` 已重构为单一 dispatch 主链：
  - workspace 准备 + memory 解析后统一调用 `Dispatcher`
  - 仅通过 runner 选择器获取具体 runner，不再维护 `run_dispatch_codex/claude/gemini/mock` 分叉函数。
- 已通过 `cargo fmt && cargo test`（61 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-027 V0.6-P0-4-ProviderMappingGuardAndDoctorFlags (Completed 2026-03-24)

任务：推进 v0.6 P0-4，校准 provider 参数映射的失败策略并把已验证 flag 组合显式暴露到 doctor/probe 输出。
验收标准：

1. provider 映射遇到未验证/不支持的参数值时，返回结构化错误而非静默降级。
2. Codex/Claude/Gemini 的 approval/permission 映射逻辑具备“显式支持范围”与失败保护。
3. `ProviderProbe` 增加已验证 flag 集合，`doctor` 报告可直接查看每个 provider 的 flag 组合。
4. 新增测试覆盖参数映射失败路径与 doctor flag 输出。
5. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增严格映射保护：
  - `codex_runner`: `runtime.approval` 仅接受已验证映射；未验证策略（如 `Ask`）立即返回 `SpecValidation` 错误。
  - `claude_runner`: `permission_mode` override 增加 allowlist 校验；`runtime.approval=Ask` 明确拒绝并返回结构化错误。
  - `gemini_runner`: `runtime.approval` 未验证策略（如 `Ask`/`AutoAcceptEdits`）明确拒绝。
- 已在 `probe::ProviderProbe` 增加 `validated_flags` 字段，并由系统 probe 按 provider 填充已验证 CLI flag 集合。
- `doctor` 渲染已新增 `validated_flags` 输出段，便于本地直接核对映射能力。
- 已新增测试：
  - `codex_runner_rejects_unvalidated_approval_policy`
  - `claude_runner_rejects_invalid_permission_mode_override`
  - `claude_runner_rejects_unvalidated_approval_policy`
  - `gemini_runner_rejects_unvalidated_approval_policy`
  - `doctor` 渲染断言覆盖 `validated_flags`
- 已通过 `cargo fmt && cargo test`（65 passed）与 `cargo run -- validate`（summary contract template: ok）。

## T-028 V0.6-P0-5-WorkspaceLifecycleCleanup (Completed 2026-03-24)

任务：推进 v0.6 P0-5，补齐 temp/worktree 生命周期清理闭环，确保成功和失败路径都不会遗留悬挂 workspace。
验收标准：

1. 新增 `runtime/cleanup` 清理模块，并接入 dispatch 生命周期。
2. `TempCopy` 与 `GitWorktree`（含 fallback）在运行结束后自动清理 workspace。
3. dispatch 失败路径（如 memory resolve 失败）同样触发补偿清理。
4. `runtime/workspace` 在准备失败时执行 best-effort 回滚，避免半创建目录残留。
5. 新增测试覆盖成功清理、失败补偿清理与 git worktree 降级清理。
6. `cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已新增 `src/runtime/cleanup.rs`：
  - `WorkspaceCleanupGuard`（基于 Drop 的生命周期清理）
  - `TempCopy/GitWorktreeFallbackTempCopy` 目录删除
  - `GitWorktree` 优先 `git worktree remove --force`，失败后回退 `remove_dir_all`
- `src/mcp/service.rs` 的 `run_dispatch` 已接入 cleanup guard，确保：
  - 正常完成后在产物采集后自动清理
  - dispatch 错误返回前自动触发清理
  - 异步任务被中断时 guard drop 也能触发清理
- `src/mcp/tools.rs` 已调整 envelope 解构，显式持有 cleanup guard 到 artifact materialization 完成。
- `src/runtime/workspace.rs` 已补准备阶段失败补偿：复制失败时 best-effort 清理半创建 workspace 目录。
- 已新增/更新测试：
  - `runtime::cleanup::tests::*`（in-place 无 guard、temp 清理、git fallback 清理）
  - `mcp::service::tests::run_dispatch_cleans_temp_workspace_after_success`
  - `mcp::service::tests::run_dispatch_error_path_cleans_temp_workspace`
  - `mcp::server::tests::run_agent_tempcopy_persists_workspace_metadata`（断言 run 后 workspace 已清理）
- 已通过 `cargo test`（70 passed）与 `cargo run -- validate`（summary contract template: ok）。

---

## Remaining Backlog (V0.6)

说明：以下为基于 `docs/mcp-subagent_tech_design_v0.6.md` 与当前代码状态整理的完整待办清单，按“大模块批次”执行，不再碎步推进。

### Batch A - 可运行命令面与主路径收口（高优先级）

## T-029 V0.6-BatchA-LocalCliCommandSurface (Completed 2026-03-24)

任务：补齐本地命令面，让不接 MCP Host 也可完成日常调试与执行。
验收标准：

1. CLI 新增并可运行：`list-agents`、`run`、`spawn`、`status`、`cancel`、`artifact`。
2. `run/spawn/status/cancel/artifact` 复用既有 runtime 执行链路，不新增一套平行实现。
3. 提供 `--json` 输出模式，便于脚本化调用。
4. `run` 最小路径可在 Mock/Codex 跑通（按本机 provider 实际可用性）。
5. 新增命令解析与关键路径测试，`cargo test` 通过。
完成记录：

- 已在 `src/main.rs` 新增本地命令：
  - `list-agents`、`run`、`spawn`、`status`、`cancel`、`artifact`
  - 统一支持 `--json` 输出模式。
- 命令执行链复用既有 MCP runtime 入口（`McpSubagentServer::{list_agents,run_agent,spawn_agent,get_agent_status,cancel_agent,read_agent_artifact}`），未引入平行执行器。
- 新增 artifact kind 解析（`summary/log/patch/json`）与默认路径决策逻辑。
- 已新增 CLI 解析测试：
  - `parses_list_agents_json_flag`
  - `parses_run_command_with_required_args`
  - `parses_artifact_kind_enum`
- 已通过 `cargo fmt && cargo test`（`src/lib.rs`: 73 passed；`src/main.rs`: 3 passed）与 `cargo run -- validate`。

## T-030 V0.6-BatchA-SummaryEnvelopeContractUpgrade (Completed 2026-03-24)

任务：完成 P0-6，将 summary contract 从“sentinel + 直接结构体”升级为 `SummaryEnvelope`。
验收标准：

1. 新增 `SummaryEnvelope`，包含 `contract_version/parse_status/summary/raw_fallback_text`。
2. `StructuredSummary` 补齐强字段：`plan_refs`，并保留 `artifacts/touched_files/verification_status` 强约束。
3. parse 失败时不伪装成功，`parse_status` 正确标记 `Degraded/Invalid`。
4. 持久化新增 `summary.raw.txt`，并在状态读取路径可回溯原始文本。
5. 新增单测覆盖 validated/degraded/invalid 三路径，`cargo test` 与 `cargo run -- validate` 通过。
完成记录：

- 已升级 `runtime::summary`：
  - 新增 `SummaryEnvelope { contract_version, parse_status, summary, raw_fallback_text }`
  - 新增 `SummaryParseStatus::{Validated,Degraded,Invalid}`
  - `StructuredSummary` 新增强字段 `plan_refs`（`serde default` 兼容旧输出）。
- `context` 与 `dispatcher` 已切换到 envelope 解析链路：
  - `ContextCompiler::parse_summary` 返回 `SummaryEnvelope`
  - runner 成功但 parse_status 非 `Validated` 时，不再标记成功结构化运行。
- 已落地 provider schema-first 参数：
  - Codex：增加 `--output-schema`（并保留 `--output-last-message`）
  - Claude：增加 `--json-schema`
  - 新增 runner 测试断言 schema flag 实际传递。
- artifact 持久化已支持 `summary.raw.txt`（当 `raw_fallback_text` 存在时写入）。
- 已更新 MCP summary 输出映射，包含 `contract_version/parse_status/plan_refs`。
- 新增/更新测试：
  - `runtime::summary::tests::{parses_valid_envelope_from_stdout,marks_invalid_when_json_is_invalid,marks_degraded_when_sentinel_missing,...}`
  - `runtime::codex_runner::tests::codex_runner_passes_output_schema_flag`
  - `runtime::claude_runner::tests::claude_runner_passes_json_schema_flag`
- 已通过 `cargo fmt && cargo test`（73 passed）与 `cargo run -- validate`。

### Batch B - Workflow 一等能力（高优先级）

## T-031 V0.6-BatchB-WorkflowSpecAndValidation (Completed 2026-03-24)

任务：引入 `WorkflowSpec` 与 gate/review/archive 等策略结构，并纳入 spec 校验。
验收标准：

1. 新增 `spec/workflow.rs` 与 `AgentSpec.workflow` 字段（可选）。
2. 支持 `stages/require_plan_when/active_plan/review_policy/knowledge_capture/archive_policy/max_runtime_depth`。
3. 校验规则覆盖：非法 stage、`max_runtime_depth` 下限、关键字段组合约束。
4. registry 与 validate 路径均覆盖 workflow 字段。
5. 新增测试覆盖加载与校验，`cargo test` 通过。
完成记录：

- 已新增 `src/spec/workflow.rs`，包含：
  - `WorkflowSpec`
  - `WorkflowGatePolicy`
  - `ActivePlanPolicy`
  - `ReviewPolicy`
  - `KnowledgeCapturePolicy`
  - `ArchivePolicy`
  - `WorkflowStageKind`
- 已在 `src/spec/mod.rs` 增加 `pub mod workflow` 与 `AgentSpec.workflow: Option<WorkflowSpec>`。
- `src/spec/validate.rs` 已新增 workflow 规则校验：
  - `max_runtime_depth > 0`
  - enabled workflow 的 stage 非空
  - gate 数值阈值不得为 0
  - `stages/allowed_stages` 去重
  - `stages` 必须落在 `allowed_stages`（当 allowlist 非空时）
- 已新增测试：
  - `rejects_zero_workflow_depth`
  - `rejects_empty_stages_for_enabled_workflow`
  - `rejects_duplicate_workflow_stages`
  - `rejects_stage_not_in_allowed_stages`
  - `accepts_workflow_with_consistent_stage_allowlist`
- 已通过 `cargo fmt && cargo test`（78 passed）与 `cargo run -- validate`。

## T-032 V0.6-BatchB-ActivePlanMemorySource (Completed 2026-03-24)

任务：将 `ActivePlan` 升级为 memory 一等来源，并支持归档记忆来源。
验收标准：

1. `MemorySource` 支持 `ActivePlan` 与 `ArchivedPlans`。
2. `resolve_memory` 在 workflow 启用时可解析 `PLAN.md`（含 excerpt 策略）。
3. provider-native memory 去重规则保持成立，不引入重复注入。
4. `PLAN.md` 缺失/损坏时返回明确错误（按 gate 条件触发）。
5. 新增测试覆盖 active plan 注入、缺失失败、native 去重，`cargo test` 通过。
完成记录：

- 已完成 `MemorySource` 扩展：`ActivePlan`、`ArchivedPlans`。
- `resolve_memory` 已支持：
  - `ActivePlan`（读取 `PLAN.md` / `.mcp-subagent/PLAN.md`）
  - `ArchivedPlans`（读取 `docs/plans/*.md` 等归档路径）
- 已更新默认 memory sources：`AutoProjectMemory + ActivePlan`。
- provider-native memory 去重保持成立（显式内联命中 native 文件时会移除 passthrough）。
- 缺失 `PLAN.md` 的失败语义已通过 workflow gate 闭环（Build/Review + gate 命中时显式报错）。
- 已新增/保留测试：
  - `active_plan_source_is_noop_when_plan_missing`
  - `active_plan_source_inlines_plan_content`
  - `archived_plans_source_inlines_existing_archives`
  - `build_stage_requires_plan_when_gate_hits`（跨任务闭环验证）
- 已通过 `cargo fmt && cargo test`（92 passed）与 `cargo run -- validate`。

## T-033 V0.6-BatchB-StageAwareDispatchAndPlanGate (Completed 2026-03-24)

任务：让 dispatcher 按阶段驱动，并在 Build/Review 前执行 plan gate。
验收标准：

1. 运行请求支持 `stage` 与 `plan_ref`，MCP + 本地 CLI 参数贯通。
2. dispatcher 引入 stage-aware 路由（至少支持 `Research/Plan/Build/Review/Archive`）。
3. 当 workflow gate 命中且无有效 `PLAN.md` 时，运行失败并返回结构化错误。
4. `max_runtime_depth` 生效，超限的 runtime-managed 派发被拒绝。
5. 新增集成测试覆盖 gate pass/fail 与 stage 路由，`cargo test` 通过。
完成记录：

- 已打通请求字段：`stage`、`plan_ref`（CLI -> DTO -> RunRequest -> snapshot）。
- dispatcher 已实现 stage-aware 路由约束：
  - 支持 `Research/Plan/Build/Review/Archive` 解析与校验；
  - 会拒绝未启用或不在 allowlist 的 stage。
- Build/Review gate 语义已收口：
  - gate 命中且无有效 `PLAN.md` 时失败并返回结构化错误；
  - 有效 plan 存在时可继续执行。
- 已新增 `max_runtime_depth` 运行时约束：
  - 从 `parent_summary` 的 `runtime_depth=` 标记推断嵌套深度；
  - 深度超限时拒绝 runtime-managed 派发。
- 已补充阶段角色优先信息注入：
  - context 模板在存在 stage 时增加 `WorkflowStage` 与 `StageRolePriority` 约束行。
- 已新增测试：
  - `rejects_stage_not_enabled_in_workflow_stages`
  - `rejects_runtime_depth_exceeding_workflow_limit`
  - `includes_stage_role_priority_when_stage_present`
  - 保留 `build_stage_requires_plan_when_gate_hits` / `build_stage_passes_when_plan_exists`
- 已通过 `cargo fmt && cargo test`（92 passed + 3 passed）与 `cargo run -- validate`。

### Batch C - 策略与 provider 分层收口（中高优先级）

## T-034 V0.6-BatchC-WorkingDirAutoPolicy (Completed 2026-03-24)

任务：实现 `WorkingDirPolicy::Auto`，让读写任务自动选择 in-place/worktree/temp-copy。
验收标准：

1. runtime policy 增加 `Auto`，并作为默认策略（符合 v0.6 拍板）。
2. 只读任务默认 `InPlace`；写任务优先 `GitWorktree`，不可用时退化 `TempCopy`。
3. workspace 记录中包含解析决策注释（why/fallback reason）。
4. 新增测试覆盖三类分支与回退，`cargo test` 通过。
完成记录：

- 已在 `spec/runtime_policy.rs` 增加 `WorkingDirPolicy::Auto`，并将默认策略切换为 `Auto`。
- 已在 `runtime/workspace.rs` 落地 auto 解析逻辑：
  - `ReadOnly` 或 `Research/Plan` 阶段默认 `InPlace`
  - 写任务优先 `GitWorktree`，不可用时保留既有 `GitWorktreeFallbackTempCopy`
- workspace notes 已补充 auto 决策说明（read-only/stage 命中原因 + fallback 说明）。
- 已新增测试：
  - `auto_policy_uses_in_place_for_read_only_task`
  - `auto_policy_prefers_worktree_for_write_task`
- 已通过 `cargo fmt && cargo test`（85 passed）与 `cargo run -- validate`。

## T-035 V0.6-BatchC-MockTierAndOllamaReserved (Completed 2026-03-24)

任务：修正 provider 分层语义，建立 Mock 一等路径，避免将 Ollama 伪装为已支持。
验收标准：

1. `Provider` 增加 `Mock`，并提供稳定 mock runner 路径。
2. `Provider::Ollama` 在未有真实 runner 前标记 `reserved`，doctor/list_agents 明确展示。
3. 无 provider binary 环境下仍可通过 Mock 路径完成本地调试。
4. probe/doctor/capability notes 与 tiers 一致。
5. 新增测试覆盖 mock 可跑与 ollama reserved 表达，`cargo test` 通过。
完成记录：

- 已将 `Provider::Mock` 路径收口为一等本地调试路径：`select_runner` 对 `Mock` 走稳定 `MockRunner`，并将相关测试夹具默认 provider 从 `Ollama` 切换到 `Mock`。
- 已将 `Provider::Ollama` 语义收口为 reserved：
  - `ensure_provider_ready` 明确拒绝 `Ollama` 运行；
  - `list_agents` 对 `Ollama` 强制 `available=false`；
  - `select_runner` 对 `Ollama` 保守返回失败 plan（防止绕过 gate 被误判为可运行）。
- 已统一 provider tier 说明：`build_capability_notes` 和 `SystemProviderProber` 均输出 tier note（Mock/Primary/Beta/Experimental/Reserved），doctor 与 list_agents 保持一致口径。
- 已新增/更新测试覆盖：
  - `list_agents_marks_ollama_reserved`
  - `run_agent_rejects_reserved_ollama_provider`
  - 既有 mock 跑通用例继续验证 `run_agent_tool_returns_structured_summary`
  - doctor 用例更新为 5 providers（含 Mock/Ollama）。
- 已通过 `cargo fmt && cargo test`（87 passed + 3 passed）与 `cargo run -- validate`。

## T-036 V0.6-BatchC-DoctorEnhancedReport (Completed 2026-03-24)

任务：扩展 `doctor` 让其承担 v0.6 要求的健康检查与策略提示。
验收标准：

1. doctor 输出 provider 能力矩阵、已验证 flags、版本/可执行状态。
2. 输出 workspace 策略成本提示（至少区分 in-place/worktree/temp-copy 建议）。
3. 输出 `PLAN.md`/project memory/archive 结构健康检查。
4. 保持文本可读和 JSON 友好（如后续接入 `--json`）。
5. 新增渲染/构建测试，`cargo test` 通过。
完成记录：

- 已扩展 `doctor` 报告结构为可序列化（JSON-friendly）对象：
  - `DoctorReport` 新增 `workspace_policy_hints`、`knowledge_layout` 字段；
  - 新增 `WorkspacePolicyHint`、`KnowledgeLayoutHealth` 结构。
- 已增强 provider 矩阵输出：
  - 文本报告从 `provider_probe` 升级为 `provider_matrix`；
  - 补齐 capability 维度（`supports_background_native / supports_native_project_memory / experimental`）；
  - 保留 `status/version/executable/validated_flags/notes`。
- 已新增 workspace 策略成本提示：
  - 对 `Auto/InPlace/GitWorktree/TempCopy` 输出使用量、成本提示与建议；
  - 使用量根据已加载 agent specs 的 `working_dir_policy` 统计。
- 已新增知识结构健康检查：
  - 检查 `PLAN.md` / `.mcp-subagent/PLAN.md`
  - 检查 `PROJECT.md` / `.mcp-subagent/PROJECT.md`
  - 检查归档计划 `docs/plans/*.md`、`archive/*.md`、`plans/archive/*.md`
  - 缺失时输出明确 warning。
- 已新增/更新测试：
  - `doctor::tests::builds_report_and_renders_key_fields`
  - `doctor::tests::checks_knowledge_layout_and_policy_usage`
- 已通过 `cargo fmt && cargo test`（88 passed + 3 passed）与 `cargo run -- validate`。

### Batch D - 状态与工件可观测性（中优先级）

## T-037 V0.6-BatchD-StateLayoutAndEventsUpgrade (Completed 2026-03-24)

任务：对齐 v0.6 持久化布局，增强 run 级可审计性。
验收标准：

1. 每次 run 落盘：`request.json/resolved-spec.json/compiled-context.md/status.json/summary.json/summary.raw.txt/events.ndjson/workspace.meta.json`。
2. `artifacts/index.json` 补齐字段：`kind/path/media_type/producer/created_at/description`。
3. 兼容读取旧 run 数据（向后兼容）。
4. 关键事件入 `events.ndjson`（probe/gate/workspace/memory/parse/cleanup）。
5. 新增重启读取与事件落盘测试，`cargo test` 通过。
完成记录：

- 已扩展 run 落盘布局（保留 `run.json` 向后兼容）并新增：
  - `request.json`
  - `resolved-spec.json`
  - `compiled-context.md`
  - `status.json`
  - `summary.json`
  - `summary.raw.txt`
  - `workspace.meta.json`
  - `events.ndjson`
  - `artifacts/index.json`
- 已升级 artifact index 结构：`ArtifactOutput` 新增 `producer`、`created_at` 字段，并在 artifact 生成阶段统一填充（runtime/agent producer 区分）。
- 已新增 memory/compiled-context 快照链路：
  - `RunRecord` 增加 `memory_resolution`、`compiled_context_markdown`
  - `run_dispatch` 输出 memory resolution 与 compiled context 内容
  - 持久化与重启加载路径均已兼容。
- 已落地关键事件写入 `events.ndjson`，覆盖：
  - `probe`
  - `gate`
  - `workspace`
  - `memory`
  - `parse`
  - `cleanup`
- 已补充兼容与落盘测试：
  - `mcp::server::run_agent_tempcopy_persists_workspace_metadata`（验证新文件布局、events、artifact index 字段）
  - `mcp::persistence::tests::loads_legacy_run_json_without_new_fields`（验证旧 run.json 兼容读取）
- 已通过 `cargo fmt && cargo test`（89 passed + 3 passed）与 `cargo run -- validate`。

### Batch E - MVP 验收与文档（中优先级）

## T-038 V0.6-BatchE-MvpSmokeAndDocs (Completed 2026-03-24)

任务：建立“本地可跑”统一验收脚本与文档，收口 v0.6 MVP。
验收标准：

1. 固化 smoke 流程：`doctor -> validate -> list-agents -> run(mock/codex) -> mcp`。
2. 明确 Claude(Beta)/Gemini(Experimental)/Ollama(Reserved) 状态声明与限制。
3. README/开发文档更新到当前命令面和配置面。
4. CI 或本地脚本可一键跑最小验收。
5. 验收清单与结果回填 TODO，形成闭环。
完成记录：

- 已新增本地一键 smoke 脚本：`scripts/smoke_v06.sh`，固化流程：
  - `doctor`
  - `validate`
  - `list-agents --json`
  - `run mock_runner --json`
  - `run codex_runner --json`（环境可用时执行；不可用时允许跳过）
  - `mcp` 启动短时校验（timeout + 初始化前退出判定）
- 已新增文档：
  - `README.md`：命令面、配置面、provider tier 声明、smoke 用法
  - `docs/mvp_smoke_v06.md`：验收清单与执行方式
- provider 状态声明已在文档与运行时口径统一：
  - `Codex=Primary`
  - `Claude=Beta`
  - `Gemini=Experimental`
  - `Mock=Stable local debug`
  - `Ollama=Reserved`
- 本地已执行并通过：
  - `./scripts/smoke_v06.sh`
  - `cargo fmt && cargo test`（89 passed + 3 passed）
  - `cargo run -- validate`

## T-039 PostV0.6-CI-Automation (Completed 2026-03-24)

任务：把本地验收链路接入 CI，避免手工漏检。
验收标准：

1. 新增 CI workflow，触发 push/pull_request。
2. CI 至少执行 `cargo fmt --check`、`cargo test`、`validate`、`smoke`。
3. 示例 specs 可在 CI 中被 validate。
完成记录：

- 已新增 `.github/workflows/ci.yml`。
- 已接入步骤：
  - `cargo fmt --check`
  - `cargo test --all-targets --locked`
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-040 PostV0.6-Examples-And-E2E-Regression (Completed 2026-03-24)

任务：补齐真实 workflow 示例并落地端到端回归测试。
验收标准：

1. 新增示例 agent spec，覆盖 `workflow + stage + plan_ref`。
2. 新增示例 workspace，包含 `PLAN.md/PROJECT.md/archived plans`。
3. 新增 e2e 测试覆盖 build pass/fail 和 depth guard。
完成记录：

- 已新增示例：
  - `examples/agents/workflow_builder.agent.toml`
  - `examples/workspaces/workflow_demo/PLAN.md`
  - `examples/workspaces/workflow_demo/PROJECT.md`
  - `examples/workspaces/workflow_demo/docs/plans/2026-03-24-bootstrap.md`
  - `examples/workspaces/workflow_demo/src/lib.rs`
- 已新增 `tests/e2e_workflow_examples.rs`，覆盖：
  - build stage + plan_ref 成功
  - plan gate 缺失失败
  - `max_runtime_depth` 超限失败

## T-041 PostV0.6-Release-Cutpoint (Completed 2026-03-24)

任务：收口发布文档与版本切点。
验收标准：

1. 版本号更新到 `0.6.0`。
2. 新增 changelog 与发布说明文档。
3. README 补齐示例与发布后使用入口。
完成记录：

- 已更新 `Cargo.toml` 版本为 `0.6.0`（同步更新 `Cargo.lock`）。
- 已新增 `CHANGELOG.md`（v0.6.0 版本摘要与 provider 状态）。
- 已新增 `docs/release_v0.6.0.md`（发布切点和打 tag 清单）。
- 已更新 `README.md`（标注 v0.6.0，并补充 examples validate 用法）。

## T-042 PostV0.6-RunnerModuleRefactor (Completed 2026-03-24)

任务：将 runner 体系封装到独立模块，简化内部命名并降低后续扩展成本。
验收标准：

1. `runtime` 下 runner 相关实现迁移到独立子模块目录。
2. runner 文件名简化为 `claude.rs/codex.rs/gemini.rs/mock.rs`。
3. 公共 trait/执行结果类型统一从单一入口导出。
4. 全量引用路径完成迁移且 `cargo test` 通过。
完成记录：

- 已新增 `src/runtime/runners/` 子模块并迁移文件：
  - `src/runtime/runners/mod.rs`
  - `src/runtime/runners/claude.rs`
  - `src/runtime/runners/codex.rs`
  - `src/runtime/runners/gemini.rs`
  - `src/runtime/runners/mock.rs`
- 已移除旧路径：
  - `src/runtime/runner.rs`
  - `src/runtime/claude_runner.rs`
  - `src/runtime/codex_runner.rs`
  - `src/runtime/gemini_runner.rs`
  - `src/runtime/mock_runner.rs`
- `src/runtime/mod.rs` 已切换为 `pub mod runners;`，并更新了 `mcp::service`、`dispatcher` 与相关测试引用。
- provider runner 构造函数命名已简化为模块内统一 `from_env()`（`runners::claude::from_env()` 等）。
- 已通过：
  - `cargo test`（92 passed + 3 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-043 PostV0.6-ReadmeBadgesLicenseAndOllamaLocalRunner (Completed 2026-03-24)

任务：收口 README 版本信息展示、仓库许可证声明，并补齐本地 Ollama 真实 runner 路径。
验收标准：

1. README 不再手写版本号，改为 GitHub release/license 徽标。
2. 仓库与 crate 元数据具备明确许可证声明，且可对应到仓库内许可证文本。
3. `Provider::Ollama` 从保留态升级为本地 runner 路径：`run_agent/spawn_agent` 可进入真实 runner 分发分支。
4. `list_agents`/provider tier 描述反映 Ollama 本地路径语义，不再强制 reserved 不可用。
5. 本地 smoke 和测试链路覆盖 Ollama 最小可跑路径（环境可选）。
完成记录：

- 已更新 `README.md`：
  - 移除显式版本文案，保留 GitHub release/license 徽标；
  - provider tier 与本地 smoke 文案已同步到 Ollama 本地 runner 路径。
- 已更新 `Cargo.toml`：新增 `license = "MIT OR Apache-2.0"`。
- 已新增许可证文件：
  - `LICENSE-MIT`
  - `LICENSE-APACHE`
- 已新增 `src/runtime/runners/ollama.rs`（真实本地 runner，支持超时/失败/模型缺失校验）并接入 `mcp::service::select_runner`。
- 已移除 `mcp` 层对 Ollama 的 reserved 硬拒绝与强制 unavailable 逻辑，`list_agents` 可按 probe 真实反映可用性。
- 已更新文档和 smoke：
  - `docs/mvp_smoke_v06.md`
  - `docs/release_v0.6.0.md`
  - `CHANGELOG.md`
  - `scripts/smoke_v06.sh`（支持 `MCP_SUBAGENT_SMOKE_OLLAMA_MODEL` 可选 smoke）。
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（96 passed + 3 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-044 V0.7-P0-ProviderMappingAndStdioDocsAlignment (Completed 2026-03-24)

任务：按 v0.7 设计文档收口 P0 映射与文档一致性（Gemini/Claude 参数映射 + stdio 命令面与可核验性声明）。
验收标准：

1. Gemini runner 不再使用 `--approval-mode plan`，改为 `ReadOnly->default / WorkspaceWrite->auto_edit / FullAccess->yolo`。
2. Claude runner 不再将 `auto` 作为公开 permission mode；override allowlist 与公开模式对齐。
3. `doctor/list-agents` 可见 provider 参数映射提示（至少覆盖 Gemini/Claude）。
4. README 明确 `mcp-subagent mcp` 为 stdio-only MCP 入口，且明确“不能保证零幻觉，只能提高可核验性”。
5. TODO 中当前命令面不再使用 `--mcp` 作为现行入口描述。
完成记录：

- 已更新 `src/runtime/runners/gemini.rs`：
  - `resolve_approval_mode` 映射改为 `default/auto_edit/yolo`。
- 已更新 `src/runtime/runners/claude.rs`：
  - `permission_mode` override allowlist 改为：
    - `default`
    - `acceptEdits`
    - `plan`
    - `dontAsk`
    - `bypassPermissions`
  - 默认映射中 `FullAccess` 改为 `bypassPermissions`，移除旧 `auto` 映射。
- 已更新 `src/probe/mod.rs`：
  - 新增 provider CLI 映射说明 notes；
  - `doctor/list-agents` 通过现有 notes 输出路径自动展示映射提示。
- 已新增/更新单测：
  - `gemini_runner_maps_readonly_to_default_approval_mode`
  - `claude_runner_rejects_legacy_auto_permission_mode_override`
  - `claude_runner_maps_full_access_to_bypass_permissions`
  - `probe::tests::gemini_mapping_notes_reflect_default_auto_edit_yolo`
  - `probe::tests::claude_mapping_notes_include_public_permission_modes`
- 已更新文档：
  - `README.md` 新增 `MCP Transport`（stdio-only）和 `Verification Model`（非零幻觉承诺）说明；
  - `TODO.md` 中现行命令面描述统一改为 `mcp` 子命令。
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（101 passed + 3 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-045 V0.7-P1-InitPresetClaudeOpusSupervisor (Completed 2026-03-24)

任务：实现 `mcp-subagent init --preset claude-opus-supervisor` 的最小可用版本，生成可直接验证的默认团队骨架。
验收标准：

1. 新增 `init` 子命令，支持 `--preset claude-opus-supervisor`。
2. 自动生成：
   - `agents/` 默认团队 specs（6 个）
   - `PLAN.md` 模板
   - `.mcp-subagent/config.toml`
   - `README.mcp-subagent.md`
3. 生成后可通过 specs 校验（至少在实现内做一次加载校验）。
4. 支持 `--force` 覆盖，默认禁止覆盖已有文件。
5. 新增单测覆盖创建成功、无 force 拒绝覆盖、force 覆盖路径。
完成记录：

- 已新增 `src/init.rs`：
  - `InitPreset` / `InitReport`；
  - `init_workspace()` 入口；
  - `claude-opus-supervisor` 模板生成逻辑；
  - 生成后调用 `load_agent_specs_from_dirs` 做即时校验。
- 已在 `src/main.rs` 接入 `init` 子命令：
  - 参数：`--preset`、`--root-dir`、`--force`、`--json`
  - 默认 preset：`claude-opus-supervisor`
  - 终端输出生成报告与下一步提示。
- 已更新 `src/lib.rs` 导出 `init` 模块。
- 已更新 `README.md` 命令面，补充 `init` 用法。
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（104 passed + 4 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`
  - `cargo run -- init --preset claude-opus-supervisor --root-dir <tmp> --json`
  - `cargo run -- --agents-dir <tmp>/agents validate`（验证生成 preset 可直接通过校验）

## T-046 V0.7-P1-SelectedFileInlineFlag (Completed 2026-03-24)

任务：新增 `--selected-file-inline`，让本地 CLI 可显式读取并内联文件内容到 selected files。
验收标准：

1. `run` / `spawn` 命令支持重复参数 `--selected-file-inline <path>`。
2. `--selected-file-inline` 会读取本地文件内容，并把内容写入 `RunAgentSelectedFileInput.content`。
3. `--selected-file` 与 `--selected-file-inline` 可混用；同一路径出现时以内联内容为准。
4. 文件读取失败时返回清晰错误，不进入运行分发。
5. 新增解析与构造单测覆盖上述行为。
完成记录：

- 已更新 `src/main.rs`：
  - `Commands::Run` / `Commands::Spawn` 新增 `selected_files_inline` 参数；
  - 新增 `build_selected_file_inputs()` 与 `resolve_inline_read_path()`；
  - `run_agent` / `spawn_agent` 改为先构建 selected files（失败即提前返回错误）。
- 已新增单测：
  - `parses_run_command_with_selected_file_inline`
  - `inline_selected_files_include_file_content`
  - `inline_selected_file_overrides_non_inline_entry`
- 已更新 `README.md`：
  - 命令面新增 `--selected-file-inline`；
  - 增加 `--selected-file` 与 `--selected-file-inline` 行为说明。
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（104 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`
  - `cargo run -- run <agent> --selected-file-inline <path> --json` + `jq .request_snapshot.selected_files`（`has_inlined_content=true`）

## T-047 V0.7-P1-WorkflowGateRemainingConditions (Completed 2026-03-24)

任务：补齐 workflow gate 其余四个条件的最小执行闭环（`cross_module/new_interface/migration/human_approval_point`）。
验收标准：

1. `require_plan_if_cross_module` 可基于 request 信号触发 plan gate。
2. `require_plan_if_new_interface` 可基于 task/task_brief 信号触发 plan gate。
3. `require_plan_if_migration` 可基于 task/task_brief 信号触发 plan gate。
4. `require_plan_if_human_approval_point` 可基于 approval 策略或 task/task_brief 信号触发 plan gate。
5. gate 命中且缺失 plan 时错误信息包含触发原因，便于观测。
6. 新增测试覆盖四类触发路径，且回归链路通过。
完成记录：

- 已更新 `src/runtime/dispatcher.rs`：
  - 新增 `collect_plan_gate_triggered_reasons()`，统一汇总 gate 触发原因；
  - 新增四类判定函数：
    - `detect_cross_module_request()`
    - `detect_new_interface_request()`
    - `detect_migration_request()`
    - `detect_human_approval_point()`
  - `enforce_workflow_gate()` 在缺失 plan 时输出 `triggered_by=...` 原因列表。
- 已新增单测：
  - `build_stage_requires_plan_when_cross_module_gate_hits`
  - `build_stage_requires_plan_when_new_interface_gate_hits`
  - `build_stage_requires_plan_when_migration_gate_hits`
  - `build_stage_requires_plan_when_human_approval_gate_hits`
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（108 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-048 V0.7-P1-StageAwareRouting (Completed 2026-03-24)

任务：落地 stage-aware routing 的最小执行约束，避免 Plan/Research 被任意 agent 乱用，并在 Review 阶段优先 reviewer agent。
验收标准：

1. `Research` / `Plan` 阶段要求 agent 具备 planning/research 角色信号（名称/描述/指令/tags）。
2. `Review` 阶段对明显 builder 型 agent 给出拒绝（提示应优先 reviewer agent）。
3. `Review` 阶段 reviewer 型 agent 可正常通过。
4. 新增测试覆盖上述放行/拒绝路径。
5. 回归链路（test/validate/smoke）通过。
完成记录：

- 已更新 `src/runtime/dispatcher.rs`：
  - `enforce_workflow_gate()` 增加 `enforce_stage_agent_routing()` 前置校验；
  - 新增角色信号判定：
    - `agent_stage_profile()`
    - `contains_any_keyword()`
    - `enforce_stage_agent_routing()`
  - `Research/Plan` 阶段校验 planning/research 信号；
  - `Review` 阶段校验 reviewer 优先，对 builder 型 profile 拒绝。
- 已新增单测：
  - `research_stage_rejects_non_planning_agent`
  - `plan_stage_allows_research_agent_profile`
  - `review_stage_rejects_builder_agent_profile`
  - `review_stage_allows_reviewer_agent_profile`
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（112 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-049 V0.7-P1-WorkflowPolicyExecutionClosure (Completed 2026-03-24)

任务：把 `spawn_policy/background_preference/max_turns/retry_policy` 从 schema 层推进到真实执行策略，补齐运行时可观测闭环。  
验收标准：

1. `spawn_policy` 与 `background_preference` 能影响 `run/spawn` 与 MCP 工具路径的真实行为。
2. `max_turns` 具备执行时终止或降级语义，并可在 run 状态中观测。
3. `retry_policy` 对可重试失败生效（次数、间隔、终态）。
4. run 快照记录“生效策略 + 来源（default/spec/override）”。
5. 回归链路（`cargo test`、`validate`、`smoke`）通过。
完成记录：

- 已将 `spawn_policy/background_preference` 落地为真实 MCP 执行路径约束：
  - `src/mcp/server.rs` 新增策略解析与执行模式决策；
  - 当策略解析为 async 且调用 `run_agent` 时直接拒绝并提示使用 `spawn_agent`；
  - 当策略解析为 sync 且调用 `spawn_agent` 时允许调用侧 override（记录 source=override）。
- 已将 `max_turns/retry_policy` 下沉到运行时执行主链：
  - `src/runtime/dispatcher.rs` 新增 attempt loop；
  - 支持 `retry_policy.max_attempts/backoff_secs`；
  - 支持 `max_turns` 对 retry 预算硬上限；
  - 增加可重试错误识别与“重试耗尽/被 max_turns 截断”终态语义。
- 已新增执行策略可观测快照并持久化：
  - `src/mcp/state.rs` 新增 `ExecutionPolicyRecord` 与 `PolicyValueSource`；
  - `run.json` 增加 `execution_policy`；
  - `events.ndjson` 新增 `policy` 事件。
- 已补充测试覆盖：
  - `src/mcp/server.rs`：
    - `run_agent_rejects_when_spawn_policy_requires_async`
    - `run_agent_rejects_when_background_prefers_async`
    - `run_agent_tempcopy_persists_workspace_metadata` 增加 execution_policy 与 policy event 断言
  - `src/runtime/dispatcher.rs`：
    - `dispatch_retries_transient_failure_and_succeeds`
    - `dispatch_stops_retry_when_max_turns_reached`
- 已通过：
  - `cargo test -q`（119 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-050 V0.7-P1-ReviewPolicyEnforcement (Completed 2026-03-24)

任务：让 `ReviewPolicy` 从声明式字段变成真实约束执行，支持高风险默认双审。  
验收标准：

1. `ReviewPolicy` 可影响 Review 阶段放行/拒绝或必经流程。
2. 高风险任务可强制双 reviewer 路径（至少 correctness + style 策略可表达）。
3. Build 与 Review 的角色隔离可观测且可测试。
4. summary/artifact 中补齐 review 证据字段或结构化记录。
5. 回归链路（`cargo test`、`validate`、`smoke`）通过。
完成记录：

- 已在 `src/runtime/dispatcher.rs` 落地 ReviewPolicy 执行约束：
  - `enforce_workflow_gate()` 新增 `enforce_review_policy()`；
  - Review 阶段会基于 `review_policy + 风险判定` 计算必须覆盖的 review track（correctness/style）；
  - 高风险任务自动提升为 dual-track 要求（correctness + style），并支持通过 `parent_summary` 继承上一次 review 证据；
  - 不满足策略时直接拒绝执行并给出可观测错误。
- 已补充 review 证据工件：
  - 新增 `src/mcp/review.rs`，在 Review 成功路径自动生成 `review/evidence.json`；
  - 证据包含：required/current/parent tracks、high_risk、dual_review_satisfied、policy 参数、summary 核验字段。
  - `src/mcp/tools.rs` 已在 `run_agent/spawn_agent` 成功路径接入 `apply_review_evidence_hook()`。
- 已新增测试覆盖：
  - `runtime::dispatcher`：
    - `review_stage_requires_dual_tracks_for_high_risk_without_parent_evidence`
    - `review_stage_accepts_dual_tracks_with_parent_summary_evidence`
  - `mcp::review`：
    - `review_stage_emits_review_evidence_artifact`
- 已通过：
  - `cargo test -q`（122 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-051 V0.7-P1-ArchiveKnowledgeCaptureHook (Completed 2026-03-24)

任务：落地自动归档 hook，在 Archive 阶段自动生成 `final summary`、`decision note` 和 `metadata index`。  
验收标准：

1. Archive 阶段成功运行后自动生成 final summary。
2. 命中 knowledge capture 触发条件时自动生成 decision note。
3. 归档 metadata index 会追加本次 run 元数据，且可被 `artifact` 命令读取。
4. 归档产物进入 run artifact index，并保留降级 warning 可观测语义。
5. 回归链路（`cargo test`、`validate`、`smoke`）通过。
完成记录：

- 已新增 `src/mcp/archive.rs`：
  - `apply_archive_hook()` 自动归档执行入口；
  - Archive 成功路径生成并落盘：
    - `<archive_dir>/<date>-<slug>-<handle>-final-summary.md`
    - `docs/decisions/<date>-<slug>-<handle>-decision-note.md`（按 policy/触发条件）
    - `<archive_dir>/index.json`（metadata index 追加写入）
  - 归档失败或配置不合法时生成 `archive/hook-warnings.txt`，不中断主运行结果。
- 已在 `src/mcp/tools.rs` 接入 hook：
  - `run_agent` 成功路径接入；
  - `spawn_agent` 后台成功路径接入。
- 归档产物同时写入：
  - 项目源目录（`workspace.source_path`）用于长期沉淀；
  - run artifacts（index + payload）用于 `artifact` 命令即时读取。
- 已新增单测：
  - `archive_stage_generates_final_summary_decision_and_metadata_index`
  - `non_archive_stage_skips_archive_hook`
  - `invalid_archive_dir_creates_warning_artifact`
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（115 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-052 V0.7-P1-McpServerDecompositionFinalPass (Completed 2026-03-24)

任务：继续拆分 `src/mcp/server.rs` 职责，降低耦合与复杂度，同时保持协议兼容。  
验收标准：

1. `server.rs` 中 capability/snapshot/io mapping/run helper 至少一类完成下沉。
2. MCP tool 对外输入输出协议保持兼容。
3. `server.rs` 复杂度可见下降（行数/职责分离）。
4. 回归链路（`cargo test`、`validate`、`smoke`）通过。
完成记录：

- 已新增 `src/mcp/helpers.rs`，下沉 `server.rs` 中的通用职责：
  - provider capability notes 组装
  - summary/output 映射
  - failed/cancelled summary 构造
  - RFC3339 时间格式化
  - run 模式策略解析（preferred/effective/label）
- 已更新模块 wiring：
  - `src/mcp/mod.rs` 新增 `helpers` 模块导出；
  - `src/mcp/server.rs` 改为聚焦服务生命周期、spec 加载、request 准备、state 管理；
  - `src/mcp/tools.rs` 改为从 `mcp::helpers` 引用通用 helper。
- 对外 MCP 协议未改动：
  - `list_agents/run_agent/spawn_agent/get_agent_status/cancel_agent/read_agent_artifact` 的入参与输出结构保持兼容。
- 已通过：
  - `cargo test -q`（122 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-053 V0.7-P1-ReadonlyGitWorktreeScopedAllow (Completed 2026-03-24)

任务：放宽 `ReadOnly + GitWorktree` 的阶段限制，仅允许在 `Research/Plan` 使用。  
验收标准：

1. `ReadOnly + GitWorktree` 在 `Research/Plan` 阶段可放行。
2. `Build/Review` 阶段继续拒绝该组合。
3. 校验与运行错误信息清晰可观测。
4. 回归链路（`cargo test`、`validate`、`smoke`）通过。
完成记录：

- 已把 `ReadOnly + GitWorktree` 的限制从“静态 spec 禁止”改为“运行时按阶段 gate”：
  - `src/spec/validate.rs` 移除全局硬拒绝；
  - `src/runtime/dispatcher.rs` 新增 `enforce_readonly_gitworktree_scope()`。
- 新 gate 语义：
  - `Research/Plan` 阶段允许 `ReadOnly + GitWorktree`；
  - `Build/Review` 阶段拒绝；
  - 缺失 stage 时拒绝并提示必须显式指定 `Research` 或 `Plan`。
- 已补测试覆盖：
  - `allows_readonly_gitworktree_combo_in_spec_validation`（spec 层放行）
  - `readonly_gitworktree_allows_research_stage`
  - `readonly_gitworktree_rejects_build_stage`
  - `readonly_gitworktree_requires_explicit_stage`
- 已通过：
  - `cargo test -q`（125 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-054 V0.7-P2-PresetPackAndPresetCatalog (Completed 2026-03-24)

任务：扩展 preset 体系，补齐项目级团队模板与目录化注册。  
验收标准：

1. 在 `claude-opus-supervisor` 之外新增多套 preset（如 codex/gemini/local/minimal）。
2. preset 具备统一注册与版本标识。
3. 每个 preset 生成后可直接 `validate`。
4. README/docs 提供初始化示例。
完成记录：

- 已扩展 `init` preset 体系：
  - `claude-opus-supervisor`（原有）
  - `codex-primary-builder`
  - `gemini-frontend-team`
  - `local-ollama-fallback`
  - `minimal-single-provider`
- 已在 `src/init.rs` 引入统一 preset 注册与版本标识：
  - `PRESET_CATALOG_VERSION = "v0.7.0"`
  - `preset_agent_templates()` 统一管理 preset -> agent templates 映射
  - `InitReport` 新增 `preset_catalog_version`
- 已补充新 preset 所需 agent 模板：
  - `CODEX_STYLE_REVIEWER_AGENT`
  - `GEMINI_STYLE_REVIEWER_AGENT`
  - `SINGLE_PROVIDER_CODER_AGENT`
- 已更新 CLI preset 枚举与映射（`src/main.rs`）：
  - `InitPresetArg` 新增四个 preset 选项
  - `print_init_report` 输出 `preset_catalog_version`
- 已更新文档初始化示例：
  - `README.md` 命令面与 preset 示例补齐。
- 已新增测试：
  - `init_supports_all_presets_and_validates`
  - 既有 `init_creates_preset_files_and_valid_specs` 增加 catalog version 断言
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（126 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-055 V0.7-P2-FinerGrainedConflictLock (Completed 2026-03-24)

任务：将并发冲突控制从仓库级串行收敛到更细粒度路径级。  
验收标准：

1. 冲突任务互斥，非冲突任务可并行。
2. 锁粒度至少可表达到目录或文件集合。
3. 异常退出可安全释放锁。
4. 并发测试覆盖冲突与非冲突路径。
完成记录：

- 已将串行锁从单 key 升级为多 key（路径粒度）：
  - `src/mcp/server.rs`：
    - `conflict_lock_key` -> `conflict_lock_keys`
    - 依据 `selected_files` 生成 `repo::top_scope` 级别 lock keys（排序去重）
  - `acquire_serialize_lock_from_state` -> `acquire_serialize_locks_from_state`
    - 支持一次性获取多把锁并按稳定顺序加锁，避免死锁。
- 已打通运行主链：
  - `src/mcp/tools.rs` 同步/异步路径均改为多锁获取；
  - `src/mcp/service.rs` `run_dispatch` 接收 `lock_keys`，`workspace` 元信息写入 `lock_keys`。
  - `src/mcp/state.rs` `WorkspaceRecord` 新增 `lock_keys`（保留 `lock_key` 兼容字段）。
- 已补并发测试：
  - `serialize_lock_blocks_until_guard_released`（冲突 scope 阻塞）
  - `serialize_lock_allows_non_conflicting_scopes`（非冲突 scope 并行）
- 已通过：
  - `cargo test -q`（127 passed + 7 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-056 V0.7-P2-DoctorJsonIdeOutput (Completed 2026-03-24)

任务：增强 `doctor --json` 稳定输出，方便 IDE/CI 消费。  
验收标准：

1. `doctor --json` 输出 schema 稳定且可测试。
2. 输出覆盖 provider 可用性、映射提示与修复建议。
3. exit code 对 CI 友好。
4. 提供样例与解析测试。
完成记录：

- 已增强 doctor 报告结构为 IDE/CI 友好的 JSON 载体：
  - `src/doctor.rs` 新增：
    - `status`（`ok|warning|error`）
    - `issues`（level/code/message/suggestion）
    - `advice`（去重修复建议列表）
  - provider 不可用、knowledge layout 缺失、agents 加载失败都会进入结构化 issues。
- 已新增 `doctor --json`：
  - `src/main.rs` 的 `Doctor` 子命令增加 `--json` flag；
  - `--json` 输出稳定序列化对象；
  - 非 `--json` 仍保持可读文本渲染。
- exit code 规则：
  - `status = error` 返回退出码 `1`
  - `status = ok|warning` 返回退出码 `0`
- 已补 CLI 解析测试：
  - `parses_doctor_json_flag`
  - 并更新 README 命令面展示 `doctor [--json]`。
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（127 passed + 9 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-057 V0.7-P2-ProviderVersionPinCompatibilityReport (Completed 2026-03-24)

任务：引入 provider 版本 pin 与兼容性报告。  
验收标准：

1. 支持 provider version pin 配置。
2. `doctor` 输出版本兼容性报告（当前版本 vs 支持矩阵）。
3. 不兼容时输出可执行修复建议。
4. 测试覆盖 pin 命中、版本漂移、禁用 pin 三类路径。
完成记录：

- 已新增 provider version pin 配置读取（基于项目 `.mcp-subagent/config.toml`）：
  - 配置段：`[provider_version_pins]`
  - 字段：`enabled`、`codex`、`claude`、`gemini`、`ollama`
- 已在 `doctor` 输出中加入兼容性报告：
  - `version_pins.enabled/source`
  - 按 provider 输出：
    - `configured_pin`
    - `detected_version`
    - `compatibility`（`matched|drift|not_detected|unpinned|disabled`）
    - `supported_policy`
    - `suggestion`
- 已将 drift/not_detected 融入 doctor issue/advice 管道，输出可执行修复建议并在 `--json` 中可消费。
- 已新增测试覆盖三类核心场景：
  - `provider_pin_report_marks_matched_when_pin_hits`
  - `provider_pin_report_marks_drift_when_pin_mismatches`
  - `provider_pin_report_marks_disabled_when_config_disabled`
- 文档已更新：
  - `README.md` 增加 provider_version_pins 配置示例。
- 已通过：
  - `cargo fmt`
  - `cargo test -q`（130 passed + 9 passed + 3 integration passed）
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v06.sh`

## T-058 V0.7-ReleaseCutpointAndCI (Completed 2026-03-24)

任务：完成 v0.7 发布切点收口（版本、文档、CI 与 smoke 基线统一）。  
验收标准：

1. 新增并纳入 `v0.7` 发布 smoke 脚本，覆盖 v0.7 新增能力的最小回归。
2. CI 默认 smoke 脚本切换到 v0.7 发布脚本。
3. 版本号与变更日志对齐到 `0.7.0`。
4. 发布文档新增 `release_v0.7.0` 与对应 smoke 清单。
5. 回归链路（`cargo test`、`validate`、`smoke`）通过。
完成记录：

- 已新增 `scripts/smoke_v07_release.sh` 并覆盖以下关键链路：
  - `doctor` / `doctor --json`
  - `validate` / `list-agents`
  - `run` on `Mock`
  - async gate（`run` fail + `spawn` pass）
  - review evidence artifact（生成 + `artifact` 读取）
  - `Codex`/`Ollama` 可选执行
  - `mcp` stdio 启动检查
- CI 已切换：
  - `.github/workflows/ci.yml` 从 `smoke_v06.sh` 改为 `smoke_v07_release.sh`。
- 发布与文档已对齐：
  - `Cargo.toml` 版本更新为 `0.7.0`
  - `CHANGELOG.md` 新增 `0.7.0` 章节
  - 新增 `docs/mvp_smoke_v07.md`
  - 新增 `docs/release_v0.7.0.md`
  - `README.md` 本地 smoke 命令更新为 `./scripts/smoke_v07_release.sh`
- 已通过：
  - `cargo fmt`
  - `cargo test -q`
  - `cargo run -- --agents-dir examples/agents validate`
  - `./scripts/smoke_v07_release.sh`

## T-059 V0.8-P0-ConnectSnippetAndOnboarding (Completed 2026-03-25)

任务：新增 `connect-snippet` 命令并升级 `init` 生成 README 为可直接复制的接入指引，收口首次成功路径。  
验收标准：

1. `mcp-subagent connect-snippet --host claude|codex|gemini` 均可执行并返回对应接入命令。
2. 输出中的 `binary`、`agents_dir`、`state_dir` 均为绝对路径，不包含示意占位符。
3. `init` 新生成 `README.mcp-subagent.md` 不再包含 `<ABSOLUTE_PATH_TO_...>`，并内置三类 host 的可执行命令。
4. 覆盖测试：CLI 解析、snippet 生成（模板/绝对路径/转义）、init README 内容；`cargo test` 全量通过。
5. 根 `README.md` 命令面与 CLI 对齐，包含 `connect-snippet`。
完成记录：

- 已同步规划文档到 v0.8 当前批次：
  - `PLAN.md` 新增 `Batch V0.8-P0 - First Success Path`，当前阶段锁定 `T-059`。
- 已新增连接片段模块：
  - `src/connect.rs` 新增 `ConnectHost`、`ConnectSnippetPaths`、`resolve_connect_snippet_paths`、`build_connect_snippet`；
  - 路径统一绝对化并做 shell 安全转义（含空格与单引号）。
- 已新增命令面：
  - `src/main.rs` 新增 `connect-snippet --host claude|codex|gemini`；
  - 复用统一配置优先级并基于 `current_exe + cwd` 解析绝对路径；
  - 已补 CLI 解析测试 `parses_connect_snippet_host`。
- 已升级 init README 模板：
  - `src/init.rs` 生成真实 Claude/Codex/Gemini 接入命令，移除占位符；
  - 增加“如何重新生成 connect snippet”指引；
  - 已补测试 `init_readme_contains_executable_connect_snippets`。
- 已同步根文档命令面：
  - `README.md` 已加入 `connect-snippet` 命令与使用示例。
- 已通过验收回归：
  - `cargo fmt`
  - `cargo test -q`（`134 + 10 + 3` tests passed）
  - `cargo run -- --help` 已显示 `connect-snippet`
  - `cargo run -- connect-snippet --host claude` 输出绝对路径命令

## T-060 V0.8-P0-SmokeV08AndCiSwitch (Completed 2026-03-25)

任务：新增 `smoke_v08.sh` 并切换 CI 默认 smoke 到 v0.8，纳入 `connect-snippet` 三 host 校验和 codex fake runner 稳定回归。  
验收标准：

1. 新增 `scripts/smoke_v08.sh`，至少覆盖：`validate`、`doctor`、`list-agents`、mock run、codex fake runner run、`mcp` boot、`connect-snippet --host claude|codex|gemini`。
2. smoke 校验 `connect-snippet` 输出使用绝对路径 `agents_dir/state_dir`，且不含占位符。
3. CI workflow 从 `smoke_v07_release.sh` 切换到 `smoke_v08.sh`。
4. 根 `README.md` 的本地 smoke 命令与当前脚本一致，消除文档漂移。
5. 本地通过：`cargo fmt`、`cargo test -q`、`./scripts/smoke_v08.sh`。
完成记录：

- 已新增 `scripts/smoke_v08.sh`：
  - 基于 v0.7 基线回归链路，新增 v0.8 必要检查；
  - 引入 `MCP_SUBAGENT_CODEX_BIN` fake binary，固定 `codex_runner` 为可重复通过路径；
  - 新增 `connect-snippet --host claude|codex|gemini` 三条校验，验证命令前缀、绝对 `agents_dir/state_dir`、无占位符。
- 已切换 CI smoke：
  - `.github/workflows/ci.yml` 从 `./scripts/smoke_v07_release.sh` 改为 `./scripts/smoke_v08.sh`；
  - step 名称同步为 `v0.8 Release Smoke`。
- 已同步文档：
  - `README.md` 的 Local Smoke 命令更新为 `./scripts/smoke_v08.sh`。
- 已通过验收回归：
  - `cargo fmt`
  - `cargo test -q`（`134 + 10 + 3` tests passed）
  - `./scripts/smoke_v08.sh` 全链路通过（含 codex fake 与 connect snippets）。

## T-061 V0.8-P0-ReleaseChainDocsAndVersionSync (Completed 2026-03-25)

任务：完成 v0.8 发布链路文档与版本同步收口，确保版本号、catalog、changelog、release 文档与 smoke 入口一致。  
验收标准：

1. 新增 `docs/release_v0.8.0.md`，包含 v0.8 scope、cut checklist、tag/push 指引。
2. 新增 `docs/mvp_smoke_v08.md`，命令与 `scripts/smoke_v08.sh` 对齐并覆盖新增校验项。
3. `CHANGELOG.md` 新增 `0.8.0` 章节，记录 connect-snippet、onboarding、smoke_v08/CI 收口。
4. `Cargo.toml` 版本更新为 `0.8.0`，`src/init.rs` 的 `PRESET_CATALOG_VERSION` 同步到 `v0.8.0`。
5. 本地通过：`cargo fmt`、`cargo test -q`、`./scripts/smoke_v08.sh`。
完成记录：

- 已新增发布文档：
  - `docs/release_v0.8.0.md`（scope、cut checklist、tag/push 指引）。
  - `docs/mvp_smoke_v08.md`（与 `smoke_v08.sh` 对齐，包含 connect-snippet 三 host 校验与 codex fake runner 说明）。
- 已更新版本与 catalog：
  - `Cargo.toml` 版本由 `0.7.0` 升级到 `0.8.0`。
  - `src/init.rs` `PRESET_CATALOG_VERSION` 升级到 `v0.8.0`，并同步测试断言。
- 已更新发布记录：
  - `CHANGELOG.md` 新增 `0.8.0 - 2026-03-25` 章节，收口 v0.8 P0 核心变更。
- 已通过验收回归：
  - `cargo fmt`
  - `cargo test -q`（`134 + 10 + 3` tests passed）
  - `./scripts/smoke_v08.sh` 全链路通过。

## T-062 V0.8-P0-RealExamplesAndReadmeOnboardingPath (Completed 2026-03-25)

任务：新增两个真实示例工作区（Rust 后端、前端 UI），并把根 README onboarding 固定为 `init -> validate -> doctor -> connect-snippet` 最短路径。  
验收标准：

1. 新增 `examples/workspaces/rust_service_refactor/`，至少包含 `PROJECT.md`、`PLAN.md`、示例代码文件与使用说明。
2. 新增 `examples/workspaces/frontend_landing_page/`，至少包含 `PROJECT.md`、`PLAN.md`、示例代码文件与使用说明。
3. 根 `README.md` 明确给出固定 onboarding 顺序：`init` -> `validate` -> `doctor` -> `connect-snippet`，并可直接复制。
4. README 示例区同步列出新增两个工作区，避免“只有 workflow_demo”的漂移。
5. 本地通过：`cargo fmt`、`cargo test -q`、`./scripts/smoke_v08.sh`。
完成记录：

- 已新增 Rust 后端真实示例工作区：
  - `examples/workspaces/rust_service_refactor/PROJECT.md`
  - `examples/workspaces/rust_service_refactor/PLAN.md`
  - `examples/workspaces/rust_service_refactor/README.md`
  - `examples/workspaces/rust_service_refactor/src/lib.rs`
- 已新增前端 UI 真实示例工作区：
  - `examples/workspaces/frontend_landing_page/PROJECT.md`
  - `examples/workspaces/frontend_landing_page/PLAN.md`
  - `examples/workspaces/frontend_landing_page/README.md`
  - `examples/workspaces/frontend_landing_page/web/index.html`
  - `examples/workspaces/frontend_landing_page/web/styles.css`
- 已收口根文档 onboarding 路径：
  - `README.md` 新增固定 `Quick Onboarding (Happy Path)` 顺序：`init -> validate -> doctor -> connect-snippet`；
  - `README.md` smoke 校验项已与 `smoke_v08.sh` 对齐；
  - `README.md` 示例列表已纳入两个新增工作区。
- 已通过验收回归：
  - `cargo fmt`
  - `cargo test -q`（`134 + 10 + 3` tests passed）
  - `./scripts/smoke_v08.sh` 全链路通过。

## T-064 V0.8-P0-CiProbeReliabilityAndNode24Warning (Completed 2026-03-25)

任务：修复 CI 中 codex provider probe 误判 MissingBinary 导致 smoke 失败，并处理 Node.js 20 actions 警告。  
验收标准：

1. `scripts/smoke_v08.sh` 中 codex fake binary 能被 provider probe 识别为 PATH 内 `codex`，避免 `run codex_runner` 前置可用性失败。
2. CI workflow 不再使用 `actions/checkout@v4`，升级到支持 Node.js 24 的版本。
3. 本地通过：`./scripts/smoke_v08.sh`。
完成记录：

- 已修复 smoke codex probe 路径：
  - `scripts/smoke_v08.sh` 将 fake binary 命名为 `codex`；
  - fake binary 新增 `--version` 输出分支，满足 provider probe 前置探测；
  - 将 `TMP_DIR` 注入 `PATH`，确保 probe 执行 `codex --version` 命中 fake binary。
- 已更新 CI action 版本：
  - `.github/workflows/ci.yml` `actions/checkout@v4` 升级为 `actions/checkout@v5`。
- 已通过验收回归：
  - `cargo test -q`（`134 + 10 + 3` tests passed）
  - `./scripts/smoke_v08.sh` 全链路通过。

## T-063 Refactoring-DisplayFormattingForEnums (Completed 2026-03-25)

任务：对于某些需要格式化的数据类型，将 `{:?}` 换成 `{}` 并支持 `Display`。
验收标准：

1. 为 `ArtifactKind`, `VerificationStatus`, `SummaryParseStatus`, `ContextMode`, `WorkingDirPolicy`, `SandboxPolicy`, `RunStatus` 实现 `std::fmt::Display`。
2. 将代码中的 `format!("{:?}", value)` 替换为 `format!("{}", value)` 或内联。
3. 通过全量测试和 clippy 检查。
完成记录：

- 已为核心数据枚举在 `summary.rs`、`runtime_policy.rs`、`dispatcher.rs` 中实现了 `Display`（复用 Debug 的原样式字面值）。
- 已批量替换 `src/mcp` 等各处的格式化宏。
- 修复了格式化降级警告。
- 基于反馈，移除了原生命周期钩子中回避性的 `#[allow(clippy::too_many_arguments)]`，通过提炼专属上下文结构体 `ArtifactCollector` 从根本上优化了 `apply_archive_hook` 和 `upsert_artifact` 的 API 设计，清除了 Clippy 警告。
- 已通过 `cargo clippy` 和 `cargo test` 全量检查。

## T-065 V0.8-P0-InitDefaultBootstrapRoot (Completed 2026-03-25)

任务：将 `init` 默认行为切换为写入独立 bootstrap 目录，避免覆盖当前仓库已有 `PLAN.md` 等文件；提供显式 `--in-place` 回退。  
验收标准：

1. `mcp-subagent init --preset ...` 在未指定 `--root-dir` 时默认写入 `./.mcp-subagent/bootstrap`。
2. 新增 `--in-place` 开关，显式指定后使用当前目录作为 root（兼容旧行为）。
3. `--in-place` 与 `--root-dir` 互斥，CLI 解析可校验。
4. README 命令面和 onboarding 文案与新默认行为一致。
5. 回归通过：`cargo fmt`、`cargo test -q`、`./scripts/smoke_v08.sh`。
完成记录：

- 已切换 `init` 默认 root 解析：
  - `src/main.rs` 新增 `resolve_init_root`，默认路径改为 `./.mcp-subagent/bootstrap`；
  - 保留显式 `--root-dir` 覆盖。
- 已新增 `--in-place` 回退开关：
  - `src/main.rs` 的 `init` 子命令新增 `--in-place`；
  - `--in-place` 与 `--root-dir` 互斥（clap 约束）；
  - 新增解析测试覆盖默认 bootstrap、in-place、互斥校验。
- 已更新 init 结果提示：
  - `src/init.rs` 的 `notes` 改为输出实际 `agents_dir/state_dir` 路径，避免默认 bootstrap 下误导 `./agents`。
- 已同步 README：
  - 命令面新增 `--in-place`；
  - Quick Onboarding 按默认 bootstrap 路径给出可复制命令；
  - 说明可用 `--in-place` 恢复旧行为。
- 已通过验收回归：
  - `cargo fmt`
  - `cargo test -q`（`134 + 13 + 3` tests passed）
  - `./scripts/smoke_v08.sh` 全链路通过。

## T-066 V0.8-P0-ProjectRootAutodiscoveryAndGitignore (Completed 2026-03-25)

任务：收口默认 bootstrap 模式的可用性，确保 `init` 后在项目根目录即可直接执行 `validate/doctor/connect-snippet`，并补充运行态目录忽略规则。  
验收标准：

1. `init` 在默认 bootstrap 模式下自动生成项目根桥接配置，且不覆盖用户已有配置（除非 `--force`）。
2. 配置解析优先识别项目根 `./.mcp-subagent/config.toml`（当文件存在时），实现“cd 到项目根即可自动识别”。
3. README 的 Happy Path 改为无需手动传 `--agents-dir/--state-dir`。
4. `.gitignore` 忽略 `.mcp-subagent` 运行态目录（state/logs/bootstrap），避免仓库噪音。
5. 回归通过：`cargo fmt`、`cargo test -q`、`./scripts/smoke_v08.sh`。
完成记录：

- 已在 `init` 默认 bootstrap 分支补齐桥接配置写入：
  - `src/main.rs` 新增 `ensure_bootstrap_bridge_config` 与模板生成逻辑；
  - 默认模式下自动生成 `./.mcp-subagent/config.toml`；
  - 既有配置默认保留，`--force` 时覆盖，且新增单测覆盖三种行为。
- 已修复配置自动识别路径：
  - `src/config.rs` 增加项目根配置优先识别（文件存在时优先于 home config）；
  - 新增路径决策单测覆盖 CLI/ENV/项目配置/home fallback。
- 已同步文档与忽略规则：
  - `README.md` Quick Onboarding 改为 `init -> validate -> doctor -> connect-snippet --host ...`；
  - `.gitignore` 新增 `.mcp-subagent/state/`、`.mcp-subagent/logs/`、`.mcp-subagent/bootstrap/`。

## T-067 V0.8-P0-InitTargetGitignoreAutopatch (Completed 2026-03-25)

任务：在默认 bootstrap `init` 路径中自动收口“目标项目 `.gitignore` 规则”，避免用户手工维护运行态忽略项。  
验收标准：

1. 默认 bootstrap 模式执行 `init` 时自动处理目标项目根 `.gitignore`。
2. 若 `.gitignore` 不存在，则自动创建并写入 mcp-subagent 运行态忽略规则。
3. 若 `.gitignore` 已存在且仅缺少部分规则，则只追加缺失项，不破坏既有内容。
4. 若已有 catch-all 规则（如 `.mcp-subagent/`），则不重复写入。
5. 回归通过：`cargo fmt`、`cargo test -q`、`./scripts/smoke_v08.sh`。
完成记录：

- 已新增目标项目 `.gitignore` 幂等补丁逻辑：
  - `src/main.rs` 新增 `ensure_project_gitignore`；
  - 默认 bootstrap `init` 后自动调用，并在 `notes` 输出“已更新/已存在无需改动”。
- 已实现规则判定与最小写入策略：
  - 支持无文件创建；
  - 支持已有内容时仅补缺失规则；
  - 支持 `.mcp-subagent/` / `.mcp-subagent/**` catch-all 场景跳过更新。
- 已补测试覆盖：
  - 缺失文件创建；
  - 部分规则补齐；
  - catch-all 已存在保持不变。

## T-068 V0.8-P0-ConnectApplyAndHostLaunch (Completed 2026-03-25)

任务：新增可直接执行的 `connect` 命令，支持一键接入 host，并可选立即启动对应 host，保留 `connect-snippet` 作为只输出文本路径。  
验收标准：

1. 新增 `mcp-subagent connect --host claude|codex|gemini`，默认直接执行对应 host 的 MCP 注册命令。
2. 新增 `--run-host`，在注册成功后直接启动对应 host CLI。
3. 保持 `connect-snippet` 兼容；两者都复用同一套路径解析与 host 参数映射。
4. 新增 CLI 解析测试与 connect 构建测试，`cargo test` 通过。
5. README 命令面与 onboarding 同步新命令，并说明 `connect-snippet` 用于仅打印命令。
完成记录：

- 已新增 `connect` 命令面并保持 `connect-snippet` 兼容：
  - `mcp-subagent connect --host claude|codex|gemini [--run-host]`；
  - `connect` 默认直接执行 host MCP 注册命令；
  - `--run-host` 在注册成功后立即启动对应 host CLI。
- 已在 connect 模块收口 host 参数映射：
  - `src/connect.rs` 新增 `ConnectInvocation`；
  - 新增 `build_connect_invocation` 和 `build_host_launch_invocation`，用于执行态命令构建；
  - `connect-snippet` 保持原输出格式，继续用于只打印命令。
- 已在主命令层新增执行逻辑：
  - `src/main.rs` 新增 `Commands::Connect` 分支；
  - 新增 `connect_command`、`resolve_connect_paths`、`run_invocation` 执行链路与错误处理。
- 已同步文档：
  - `README.md` 命令面和 Quick Onboarding 改为优先 `connect --host ...`；
  - `src/init.rs` 生成模板新增“直接接入”示例，并保留 snippet-only 指令。
- 已通过回归：
  - `cargo test -q`（`141 + 21 + 3` tests passed）
  - `./scripts/smoke_v08.sh`（使用 provider stub 避免本机外部 CLI probe 阻塞）通过。

## T-069 V0.8-P0-CodexOutputSchemaStrictCompat (Completed 2026-03-25)

任务：修复 Codex CLI `--output-schema` 在 OpenAI 严格响应格式下的兼容性，避免 `invalid_json_schema` 导致子代理失败。  
验收标准：

1. Codex runner 输出 schema 满足当前 strict 要求：对象 `properties` 中所有键都在 `required` 中。
2. 失败日志摘要优先展示真正错误行（如 `ERROR:`/`invalid_json_schema`），不再只显示 banner。
3. 新增单测覆盖 schema 规范化与 stderr 错误行提取逻辑。
4. `cargo test` 通过。
完成记录：

- 已修复 Codex `--output-schema` strict 兼容：
  - `src/runtime/runners/codex.rs` 新增 schema 规范化流程；
  - 对象 schema 会自动将 `properties` 中全部字段写入 `required`，满足 strict 响应格式要求（含 `media_type`）。
- 已优化错误摘要提取：
  - 失败时优先提取 `ERROR:` / `invalid_json_schema` 行，避免只显示 banner 导致误判。
- 已补充测试覆盖：
  - `strict_schema_marks_all_properties_as_required`
  - `schema_json_includes_media_type_in_required_list`
  - `summarize_stderr_prefers_error_lines`
- 已通过回归：
  - `cargo test -q`（`144 + 21 + 3` tests passed）

## T-070 V0.8-P0-CleanCommandForRunLogCache (Completed 2026-03-25)

任务：新增 `clean` 命令，清理历史 run 日志与缓存目录，降低 `state_dir` 噪音和体积。  
验收标准：

1. 新增 `mcp-subagent clean`，默认清理 `state_dir/runs`、`state_dir/server.log`、`state_dir/logs`。
2. 新增 `--dry-run` 预览模式，不实际删除但返回将删除目标。
3. 新增 `--all`，可清空整个 `state_dir`。
4. 新增 CLI 解析与 clean 行为测试，`cargo test` 通过。
5. README 命令面和使用说明同步 `clean`。
完成记录：

- 已新增 `clean` 命令面：
  - `mcp-subagent clean [--all] [--dry-run] [--json]`；
  - 默认模式清理 `state_dir/runs`、`state_dir/server.log`、`state_dir/logs`。
- 已实现清理执行与报告输出：
  - 新增 `clean_state_dir`、`estimate_path_size`、`print_clean_report`；
  - 支持 dry-run 预览 `would_remove`；
  - 支持 `--all` 清空整个 `state_dir`。
- 已补测试覆盖：
  - CLI 解析：`parses_clean_command_flags`
  - 清理行为：默认清理、dry-run 不删除、all 模式清空 state_dir。
- 已同步文档：
  - `README.md` 命令面新增 `clean`；
  - 新增 Cleanup 使用示例与三种模式说明。
- 已通过回归：
  - `cargo test -q`（`144 + 25 + 3` tests passed）。

## T-071 V0.8-P0-SummaryParseRobustnessForProviderOutput (Completed 2026-03-25)

任务：修复 provider 输出包含提示词占位 sentinel 或仅返回裸 JSON 时的 summary 解析失败，避免任务实际成功却被状态机误判为 failed。  
验收标准：

1. 当 stdout/stderr 出现多个 sentinel 区块时，解析器可跳过占位块并命中后续有效 JSON。
2. 当 provider 未返回 sentinel、但返回合法 `SummaryEnvelope` 或 `StructuredSummary` JSON 时，可正确解析为 `Validated`。
3. 保持现有语义：完全无 JSON 仍为 `Degraded`，仅有非法 JSON 时为 `Invalid`。
4. 新增单测覆盖占位 sentinel + 有效 JSON、双 sentinel 首块占位、无 sentinel 裸 JSON 三条路径。
5. 回归通过：`cargo test -q`。
完成记录：

- 已增强 summary 解析路径：
  - `src/runtime/summary.rs` 从“单一 sentinel 提取”升级为“多候选扫描 + 逐个尝试解析”；
  - 新增多 sentinel 区块遍历；
  - 新增原始输出 JSON 对象提取（支持无 sentinel 裸 JSON 输出）。
- 已保持失败语义兼容：
  - 找到候选但全部解析失败 -> `Invalid`；
  - 完全找不到候选 JSON -> `Degraded`。
- 已新增测试：
  - `parses_valid_json_without_sentinels`
  - `parses_late_valid_json_after_placeholder_sentinel_block`
  - `parses_second_sentinel_block_when_first_is_placeholder`
- 已通过回归：
  - `cargo test -q`（`147 + 25 + 3` tests passed）。

## T-072 V0.9-P0-DelegationMinimalAndBestEffortResultSemantics (Completed 2026-03-25)

任务：启动 v0.9 第一批重构，先把默认策略和成功判定语义切到“轻委派 + native-first”。  
验收标准：

1. `RuntimePolicy` 新增 `delegation_context/native_discovery/output_mode/parse_policy`，默认分别为 `minimal/minimal/both/best_effort`。
2. `default_memory_sources()` 移除 `ActivePlan`，默认仅 `AutoProjectMemory`。
3. provider 进程成功且 `parse_policy=best_effort` 时，即使 summary 归一化是 `Invalid/Degraded` 也不判 hard fail。
4. CLI 新增 `submit` 命令，行为与 `spawn` 一致（兼容保留 `spawn`）。
5. `init` 预设模板不再默认写入 `active_plan` memory source，并支持 `claude-opus-supervisor-minimal`。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 `src/spec/runtime_policy.rs`：
  - 新增 `DelegationContextPolicy`、`NativeDiscoveryPolicy`、`OutputMode`、`ParsePolicy`；
  - `RuntimePolicy` 增加对应字段；
  - 默认值切到 `minimal/minimal/both/best_effort`；
  - 默认 `memory_sources` 改为 `["auto_project_memory"]`；
  - 新增默认值单测 `runtime_policy_defaults_follow_v09_minimal_profile`。
- 已更新 `src/runtime/dispatcher.rs`：
  - `assess_attempt_outcome` 引入 `parse_policy`；
  - `best_effort` 模式下，provider 成功 + parse 非 Validated 会标记整体 `Succeeded`（保留 parse_status）；
  - `strict` 模式保持原失败语义；
  - 新增两条测试覆盖 best-effort/strict 分流。
- 已更新 `src/runtime/summary.rs`：
  - 对“provider 返回业务 JSON 但非 SummaryEnvelope”场景，支持包装为可消费的 Validated summary；
  - 新增对应测试 `wraps_json_payload_inside_sentinel_as_validated`。
- 已更新命令面与模板：
  - `src/main.rs` 新增 `submit` 子命令（`spawn` 等价别名）；
  - `src/main.rs` 的 `init` 默认 preset 切到 `claude-opus-supervisor-minimal`；
  - `src/init.rs` 新增 `claude-opus-supervisor-minimal` preset 名称并纳入 preset 生成；
  - `src/init.rs` 预设模板 `memory_sources` 默认移除 `active_plan`；
  - `README.md` 命令面同步新增 `submit` 和新 preset 名称。
- 已通过回归：
  - `cargo test -q`（`151 + 27 + 3` tests passed）。

## T-073 V0.9-P0-GeminiNativeDiscoveryIsolationAndFallback (Completed 2026-03-25)

任务：把 `native_discovery` 从配置字段落到 Gemini runner 实际执行路径，解决子代理被 ambient workspace skills 污染的问题。  
验收标准：

1. Gemini runner 支持 `native_discovery` 策略分流：`inherit/allowlist` 维持原行为，`minimal` 使用隔离 launch cwd，`isolated` 使用临时 HOME/XDG 隔离。
2. `minimal`/`isolated` 模式下，`--include-directories` 仍指向任务工作目录，避免丢失目标目录可见性。
3. `isolated` 模式遇到认证类失败时自动回退到 `minimal` 并保留可审计提示，不直接硬失败。
4. `init` 模板更新为 v0.9 默认策略：预设中显式写入 `delegation_context/output_mode/parse_policy`，Gemini 角色默认 `native_discovery = "isolated"`。
5. README Happy Path 改为 `claude-opus-supervisor-minimal`。
6. `cargo test -q` 通过。
完成记录：

- 已改造 `src/runtime/runners/gemini.rs`：
  - 新增 `DiscoveryLaunch` 与 `prepare_discovery_launch`；
  - `minimal` 策略使用临时 launch cwd，避免 workspace 层 skills 自动发现；
  - `isolated` 策略额外注入临时 `HOME/XDG_*` 环境；
  - 新增认证类失败检测与自动回退 `minimal`；
  - 新增回退 stderr 合并提示，保留初次失败证据。
- 已补充 Gemini runner 测试覆盖：
  - `gemini_runner_minimal_discovery_uses_isolated_launch_cwd`
  - `gemini_runner_isolated_discovery_falls_back_to_minimal_on_auth_error`
  - 现有 runner 测试同步到显式 `native_discovery` 样例。
- 已同步模板与文档：
  - `src/init.rs` 各预设 runtime 段落新增 v0.9 策略字段；
  - Gemini 角色默认 `native_discovery = "isolated"`；
  - `README.md` Quick Onboarding preset 更新为 `claude-opus-supervisor-minimal`。
- 已通过回归：
  - `cargo test -q runtime::runners::gemini::tests`
  - `cargo test -q`（`153 + 27 + 3` tests passed）。

## T-074 V0.9-P0-RunObservabilityCommandsAndUsageSurface (Completed 2026-03-25)

任务：收口 v0.9 可观测性和命令体验，新增顺手的 run 查看命令面并在 `show` 输出 usage/duration/provider_exit_code。  
验收标准：

1. CLI 新增 `ps/show/result/logs/watch`。
2. `show` 输出包含 `status/provider/model/normalization_status/duration_ms/provider_exit_code/retries`。
3. `result` 支持 `--raw` / `--normalized` / `--summary`（默认 summary）。
4. `logs` 支持 `--stdout` / `--stderr` 并可 `--json` 输出。
5. `watch` 支持 `--interval-ms` 与 `--timeout-secs`，终态自动退出。
6. README 命令面同步新命令。
7. `cargo test -q` 通过。
完成记录：

- 已扩展 `src/main.rs` 命令面：
  - 新增 `Commands::Ps/Show/Result/Logs/Watch`；
  - 新增对应 dispatch 分支与执行函数。
- 已新增 run 持久化读取与观测模型：
  - 新增 `StoredRunRecord` 系列结构用于从 `state_dir/runs/<id>/run.json` 读取；
  - 新增 `UsageStatsOutput`，输出 `duration_ms/provider_exit_code/retries/token_source/estimated_*`；
  - 新增 `RunListEntry/RunShowOutput/RunResultOutput/RunLogsOutput`。
- 已实现命令行为：
  - `ps` 按 `updated_at` 倒序列运行记录；
  - `show` 输出运行摘要与 usage；
  - `result` 支持 raw/normalized/summary 三种视图；
  - `logs` 支持 stdout/stderr 选择；
  - `watch` 轮询 run.json 并在终态退出，支持超时。
- 已同步文档：
  - `README.md` 命令面新增 `ps/show/result/logs/watch`。
- 已通过回归：
  - `cargo test -q`（`153 + 32 + 3` tests passed）。

## T-075 V0.9-P1-McpRunObservabilityToolsParity (Completed 2026-03-25)

任务：在 MCP 工具面补齐 run 观测与结果读取能力，新增 `list_runs/get_run_result/read_run_logs/watch_run`，让 host 无需拼接 `status + artifact`。
验收标准：

1. MCP tool 列表包含 `list_runs/get_run_result/read_run_logs/watch_run`。
2. `list_runs` 可按最近更新时间返回 run 列表，支持 `limit`。
3. `get_run_result` 返回 `native_result + normalized_result + usage`，并包含 `normalization_status/provider_exit_code/retries`。
4. `read_run_logs` 支持 `stream=stdout|stderr|both`，默认 `both`。
5. `watch_run` 支持 `interval_ms/timeout_secs`，终态返回 `terminal=true`，超时返回 `timed_out=true`。
6. 增加 MCP 端到端测试覆盖新增工具链路，并保持 `cargo test -q` 通过。
完成记录：

- 已扩展 MCP DTO：
  - `src/mcp/dto.rs` 新增 `ListRuns* / GetRunResult* / ReadRunLogs* / WatchRun*` 输入输出结构；
  - 新增 `RunUsageOutput`，统一 usage/duration/provider_exit_code/retries 结果面。
- 已扩展 MCP tools 实现：
  - `src/mcp/tools.rs` 新增 `list_runs/get_run_result/read_run_logs/watch_run`；
  - `list_runs` 支持 `limit` 并按 `updated_at` 倒序；
  - `get_run_result` 同时返回 `native_result + normalized_result + usage`；
  - `read_run_logs` 支持 `stream=stdout|stderr|both`；
  - `watch_run` 支持 `interval_ms/timeout_secs`，返回 `terminal/timed_out`。
- 已同步导出与文档：
  - `src/mcp/server.rs` 导出新增 MCP DTO 类型；
  - `README.md` MCP tools 列表加入四个新工具。
- 已补端到端覆盖：
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 覆盖新增工具调用链路（list/result/logs/watch）。
- 已通过回归：
  - `cargo test -q`（`153 + 32 + 3` tests passed）。

## T-076 V0.9-P1-ResultJsonStableSchema (Completed 2026-03-25)

任务：固定 `result --json` 输出 schema，并与 MCP `get_run_result` 结果模型对齐，减少 host 端解析分支。  
验收标准：

1. CLI `result --json` 输出包含固定字段：`contract_version/view/normalization_status/native_result/normalized_result/usage/provider_exit_code/retries`。
2. MCP `get_run_result` 输出增加 `contract_version`，字段语义与 CLI 对齐。
3. 新增测试覆盖固定 schema 的关键字段存在性（至少覆盖 CLI 序列化和 MCP e2e 返回）。
4. README 命令说明补充固定 schema 约定（简要说明）。
5. `cargo test -q` 通过。
完成记录：

- 已固定 CLI `result --json` 契约：
  - `src/main.rs` 的 `RunResultOutput` 新增固定字段 `contract_version/view/summary/provider_exit_code/retries/usage/error_message/artifact_index`；
  - `normalization_status` 改为稳定字符串（无 summary 时输出 `NotAvailable`）；
  - 契约版本固定为 `mcp-subagent.result.v1`。
- 已对齐 MCP `get_run_result`：
  - `src/mcp/dto.rs` 的 `GetRunResultOutput` 新增 `contract_version`；
  - `normalization_status` 改为稳定字符串；
  - `src/mcp/tools.rs` 返回与 CLI 对齐的契约版本和状态语义。
- 已补测试：
  - `src/main.rs::result_json_schema_contains_stable_fields` 覆盖 CLI JSON 固定字段；
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 增加 `contract_version` 断言。
- 已同步文档：
  - `README.md` 增加 `result --json` / `get_run_result` 使用同一 `contract_version` 说明。
- 已通过回归：
  - `cargo test -q`（`153 + 33 + 3` tests passed）。

## T-077 V0.9-P1-PlanSectionSelectorRuntimeSupport (Completed 2026-03-25)

任务：落地 `PlanSection` 的 section selector，从“策略枚举”升级为可执行行为：必须配置 selector，运行时仅注入目标 section。  
验收标准：

1. `RuntimePolicy` 新增 `plan_section_selector` 字段，并保持向后兼容默认值。
2. `validate` 对 `delegation_context=plan_section` 强制要求 selector 非空。
3. memory resolver 在 `delegation_context=plan_section` 时，从 `PLAN.md` 提取对应 section（按 heading 选择）并注入 memory，而非全量 plan。
4. `init --preset claude-opus-supervisor*` 生成的 `correctness-reviewer` 默认携带 `plan_section_selector`。
5. 新增单测覆盖：校验失败路径、selector 提取成功路径、selector 未命中失败路径。
6. `cargo test -q` 通过。
完成记录：

- 已扩展 runtime policy：
  - `src/spec/runtime_policy.rs` 新增 `plan_section_selector: Option<String>`；
  - 默认值保持 `None`，兼容旧 spec。
- 已落地校验规则：
  - `src/spec/validate.rs` 在 `delegation_context=plan_section` 时强制要求 `plan_section_selector` 非空；
  - 新增校验测试覆盖缺失 selector 失败、存在 selector 通过。
- 已落地运行时行为：
  - `src/runtime/memory.rs` 在 `delegation_context=plan_section` 时从 `PLAN.md` / `.mcp-subagent/PLAN.md` 提取目标 heading section；
  - 支持 exact/contains（不区分大小写）匹配 heading；
  - selector 未命中时返回明确错误而非注入全量 plan。
- 已同步预设与可观测快照：
  - `src/init.rs` 的 `correctness-reviewer` 模板新增 `plan_section_selector = "Acceptance Criteria"`；
  - `src/mcp/state.rs` 的 `RunSpecSnapshot` 新增 `delegation_context/plan_section_selector`（含旧 run 兼容默认）。
- 已补单测：
  - `src/runtime/memory.rs` 新增 section 提取成功/未命中失败测试；
  - `src/spec/validate.rs` 新增 plan_section selector 校验测试。
- 已同步设计文档示例：
  - `docs/mcp-subagent_tech_design_v0.9.md` 的 `correctness-reviewer` 示例补 `plan_section_selector`。
- 已通过回归：
  - `cargo test -q`（`157 + 33 + 3` tests passed）。

## T-078 V0.9-P1-ReviewerDefaultAcceptanceCriteriaInjection (Completed 2026-03-25)

任务：让 reviewer 路径默认附带 plan acceptance criteria，减少“审查与计划标准脱节”的情况。  
验收标准：

1. dispatch 阶段在 review 相关任务上自动吸收 `plan_section` memory 中的 checklist 条目并追加到 `request.acceptance_criteria`（不覆盖用户显式 criteria）。
2. 注入策略要去重，避免重复标准。
3. 无 `plan_section` 内容时保持原行为，不报错。
4. 新增测试覆盖“review + plan_section”时 compiled prompt 含 plan 条目。
5. `cargo test -q` 通过。
完成记录：

- 已在 dispatch 阶段落地 reviewer 默认标准注入：
  - `src/mcp/service.rs` 新增 `attach_plan_section_acceptance_criteria`；
  - 当任务为 review 相关（`delegation_context=plan_section` / `stage=review` / `tag=review`）时，从 `plan_section:*` memory 提取 checklist 项并追加到 `request.acceptance_criteria`。
- 已实现提取与去重逻辑：
  - 支持 markdown `-/*/+` 与 `1. ` 列表项提取；
  - 通过不区分大小写比较避免重复注入。
- 已保持兼容行为：
  - 无 `plan_section` memory 或无 checklist 时保持原行为，不报错。
- 已补测试：
  - `src/mcp/service.rs::run_dispatch_attaches_plan_section_acceptance_criteria_for_reviewer` 验证 compiled prompt 出现 plan 衍生标准。
- 已通过回归：
  - `cargo test -q`（`158 + 33 + 3` tests passed）。

## T-079 V0.9-P1-ShowCommandColorizedCompactOutput (Completed 2026-03-25)

任务：实现 `show` 的彩色简洁文本输出，提高人工查看效率；`--json` 保持现有契约不变。  
验收标准：

1. `show` 默认文本输出改为紧凑单页，包含状态徽章、provider/model、normalization、duration、exit code、retries、summary/error。
2. 终端支持且未设置 `NO_COLOR` 时输出 ANSI 颜色；`NO_COLOR` 或非终端时自动退化为纯文本。
3. `show --json` 输出字段和语义保持不变。
4. 新增测试覆盖彩色徽章和无彩色降级路径。
5. `cargo test -q` 通过。
完成记录：

- 已实现 `show` 文本渲染器：
  - `src/main.rs` 新增 `render_show_run_text`，输出紧凑单页信息（状态徽章、provider/model、normalization、duration、exit code、retries、summary/error）。
  - `show` 默认文本路径改为统一调用该渲染器；`--json` 路径未改动。
- 已实现颜色策略：
  - 新增 `should_use_color_output`（`NO_COLOR`、`TERM=dumb`、非终端自动禁用）；
  - 状态徽章按状态着色（succeeded/failed/running/timed_out/cancelled）。
- 已补测试：
  - `show_renderer_emits_color_badge_when_enabled`
  - `show_renderer_is_plain_when_color_disabled`
- 已同步文档：
  - `README.md` 增加 `show` 的彩色输出与 `NO_COLOR/--json` 说明。
- 已通过回归：
  - `cargo test -q`（`158 + 35 + 3` tests passed）。

## T-080 V0.9-P1-VersionedResultContractDocAndOnboardingEntry (Completed 2026-03-25)

任务：发布版本化结果契约文档，给 CLI/MCP 集成方一个固定、可引用、可迁移的对接入口。  
验收标准：

1. 新增 `docs` 文档，明确 `mcp-subagent.result.v1` 的字段、类型、语义和兼容规则。
2. 文档覆盖两条接口：CLI `result --json` 与 MCP `get_run_result`，并给出差异字段对照。
3. README 增加契约文档入口，避免集成方只靠源码反推。
4. TODO/PLAN 同步到 T-080 状态。
5. `cargo test -q` 通过（确保文档变更未引入回归）。
完成记录：

- 已新增版本化契约文档：
  - 新建 `docs/result_contract_v1.md`，固定声明 `mcp-subagent.result.v1`；
  - 文档覆盖共享字段、CLI/MCP 差异字段、`usage` 子结构、兼容策略与最小示例。
- 已补 README 入口：
  - `README.md` 在 `result --json` 契约说明处增加文档链接，集成方可直接跳转。
- 已同步流程状态：
  - `PLAN.md` / `TODO.md` 更新为 T-080 完成。
- 已通过回归：
  - `cargo test -q`（`158 + 35 + 3` tests passed）。

## T-081 V0.9-P1-NativeUsageCaptureAndFallbackMerge (Completed 2026-03-25)

任务：把 usage 结果面升级为 native-first：优先采集 provider 原生 token usage，估算值仅做兜底。  
验收标准：

1. 运行时新增 native usage 解析与存储，`run.json` 持久化 usage 字段。
2. CLI `show/result --json` 的 usage 计算优先使用 native usage，不可得时回落估算。
3. MCP `get_run_result` 的 usage 计算同步 native-first，`token_source` 支持 `native|estimated|mixed|unknown`。
4. 新增测试覆盖 native usage 解析关键路径（至少 codex tokens used 解析与无 usage 场景）。
5. 文档契约更新 `token_source` 可选值。
6. `cargo test -q` 通过。
完成记录：

- 已新增 native usage 解析链路：
  - 新增 `src/runtime/usage.rs`，支持通用 token 字段解析与 Codex `tokens used` 解析；
  - `DispatchResult` 新增 `native_usage`，在 dispatch 完成时随 stdout/stderr 一并产出。
- 已落地持久化与读取：
  - `src/mcp/state.rs` 的 `RunRecord`/`PersistedRunRecord` 新增 `usage` 字段；
  - `src/mcp/persistence.rs` 已支持从 `run.json` 回填 usage。
- 已完成 CLI/MCP usage 结果面收口：
  - `src/main.rs` 与 `src/mcp/tools.rs` 的 usage 计算改为 native-first；
  - `token_source` 细分为 `native|mixed|estimated|unknown`，native 不足时按字段级别回落估算。
- 已补测试与契约文档：
  - `src/runtime/usage.rs` 新增 native usage 解析测试（含 Codex multiline 与无 usage 场景）；
  - `src/main.rs` 新增 usage source 选择测试；
  - `docs/result_contract_v1.md` 已更新 `token_source` 可选值。
- 已通过回归：
  - `cargo test -q`（`161 + 37 + 3` tests passed）。

## T-082 V0.9-P2-RunTimelineEventStreamCli (Completed 2026-03-25)

任务：新增 run timeline 命令，直接读取并展示 `events.ndjson`，降低排障时手工翻目录成本。  
验收标准：

1. CLI 新增 `timeline <handle_id>` 子命令，默认文本输出事件流，并支持 `--json`。
2. `timeline` 支持 `--event <name>` 过滤事件类型（例如 `parse`/`workspace`）。
3. 当 run 或事件文件不存在时返回清晰错误，不静默成功。
4. 新增单测覆盖命令解析与事件文件读取/过滤路径。
5. README 命令面同步新增 `timeline`。
6. `cargo test -q` 通过。
完成记录：

- 已新增 CLI 命令面：
  - `src/main.rs` 新增 `timeline <handle_id> [--event ...] [--json]`；
  - 默认文本输出事件流，`--json` 输出结构化 `RunTimelineOutput`。
- 已落地事件读取与过滤：
  - 新增 `run_events_path/load_run_events/filter_timeline_events`；
  - 从 `state/runs/<id>/events.ndjson` 逐行解析，支持按 `--event` 过滤。
- 已增强错误语义：
  - run 或事件文件缺失、行级 JSON 损坏都会返回明确 `timeline failed: ...` 错误。
- 已补测试与文档：
  - 新增 `parses_timeline_command_flags`；
  - 新增 `load_run_events_and_filter_by_event_name`；
  - `README.md` 命令面新增 `timeline`。
- 已通过回归：
  - `cargo test -q`（`161 + 39 + 3` tests passed）。

## T-083 V0.9-P2-ProviderUsagePrecisionParsing (Completed 2026-03-25)

任务：增强 provider usage 解析精度，优先识别更多真实 usage 形态，减少 `estimated` 覆盖范围。  
验收标准：

1. 扩展 native usage 解析规则，覆盖 snake_case/camelCase 的常见 usage key（例如 `input_tokens`、`promptTokenCount`）。
2. 扩展 token 关键词匹配，覆盖 `tokens_used` 及 `tokens used` 的跨行/同行形态。
3. 保持 native-first 结果面不变，无法识别时继续回退估算。
4. 新增单测覆盖至少两类新增格式（JSON key 与 camelCase key）及无效文本场景。
5. `cargo test -q` 通过。
完成记录：

- 已扩展 usage key 解析白名单：
  - `src/runtime/usage.rs` 新增 snake_case / 空格分隔 / camelCase 常见 key；
  - 输入侧覆盖 `input_tokens/prompt_tokens/promptTokenCount` 等；
  - 输出侧覆盖 `output_tokens/completion_tokens/candidatesTokenCount` 等；
  - 总量侧覆盖 `total_tokens/totalTokenCount`，Codex 额外覆盖 `tokens used/tokens_used`。
- 已增强值提取策略：
  - 采用 key 边界判断 + key 后缀近邻解析，避免从无关上下文误吸数字；
  - 显式忽略 `null` 值并保留 fallback 估算路径；
  - 数字解析支持 `40,005` 与 `40_005` 形态。
- 已补测试覆盖新增格式：
  - `parses_usage_from_json_keys`
  - `parses_usage_from_camel_case_token_counts`
  - `does_not_treat_null_as_numeric_usage`
  - 保留并通过原有 `tokens used` 与空文本场景测试。
- 已通过回归：
  - `cargo test -q runtime::usage::tests`
  - `cargo fmt && cargo test -q`（`164 + 39 + 3` tests passed）。

## T-084 V0.9-P2-PerProviderAmbientIsolationDiagnostics (Completed 2026-03-25)

任务：在 `doctor` 增加 per-provider ambient isolation 诊断，让 Gemini/Claude/Codex 的 discovery 噪声风险可见可排障。  
验收标准：

1. `doctor --json` 输出新增 ambient isolation 结构，包含 provider 级 `native_discovery` 分布与风险等级。
2. 诊断输出包含 skill roots 探测与 workspace-visible skill conflict 列表。
3. 当存在高风险 discovery 配置或冲突时，`issues/advice` 给出明确建议。
4. 文本模式 `doctor` 同步渲染上述诊断信息。
5. 新增测试覆盖冲突识别与渲染关键字段。
6. `cargo test -q` 通过。
完成记录：

- 已扩展 `DoctorReport`：
  - `src/doctor.rs` 新增 `ambient_isolation` 字段及配套 DTO（provider profile、skill roots、skill conflicts）。
- 已实现 per-provider 风险分析：
  - 基于已加载 agent spec 统计 `native_discovery` 模式分布；
  - 对 `gemini/claude/codex` 输出 `ambient_risk` 与推荐动作；
  - `inherit/allowlist` 在 Gemini 且存在冲突时升级为 `high`。
- 已实现 skill roots 与冲突检测：
  - 探测 workspace/user 的 `.agents/skills` 与 `.gemini/skills`；
  - 仅将“涉及 workspace root 的重名 skill”标记为冲突，避免纯用户态噪声。
- 已接入健康判定：
  - 新增 `provider_*_ambient_discovery` 与 `ambient_skill_conflicts` warning；
  - `advice` 自动收敛到隔离建议。
- 已同步文本输出与文档：
  - `render_doctor_report` 增加 `ambient_isolation` 段落；
  - `README.md` 增加 `doctor --json` 新诊断字段说明。
- 已补测试：
  - `builds_report_and_renders_key_fields` 增加 `ambient_isolation` 断言；
  - 新增 `ambient_isolation_detects_workspace_visible_skill_conflict_for_gemini`。
- 已通过回归：
  - `cargo test -q`（`165 + 39 + 3` tests passed）。

## T-085 V0.9-P2-RetryClassificationOutputOnly (Completed 2026-03-25)

任务：增加 retry 分类可观测性输出，仅暴露分类与原因，不改变现有重试执行行为。  
验收标准：

1. 运行时记录最终尝试的 retry 分类（`retryable|non_retryable|unknown`）和分类原因。
2. `run.json` 与 `events.ndjson` 持久化该分类信息。
3. CLI `show/result --json` 和 MCP `get_run_result` 输出该分类信息。
4. 不修改既有重试决策逻辑（是否重试、退避、次数保持原样）。
5. 新增测试覆盖分类输出与持久化读取路径。
6. `cargo test -q` 通过。
完成记录：

- 已在运行时输出 retry 分类信息（不改执行）：
  - `src/runtime/dispatcher.rs` 新增 `RetryClassification`（`retryable|non_retryable|unknown`）；
  - `RunMetadata` 新增 `retry_classification/retry_classification_reason`；
  - 分类逻辑在失败文案解析、strict parse、timeout/cancel 等路径均会写入原因文本。
- 已完成持久化与事件输出：
  - `src/mcp/state.rs` 的 `RunRecord/PersistedRunRecord` 新增 `retry_classification`；
  - `src/mcp/persistence.rs` 将其写入 `run.json`；
  - `events.ndjson` 新增 `retry_classification` 事件（含分类与原因）。
- 已完成 CLI/MCP 结果面透传：
  - `src/main.rs` 的 `show/result --json` 新增 `retry_classification/classification_reason`；
  - `src/mcp/dto.rs` + `src/mcp/tools.rs` 的 `get_run_result` 新增同名字段；
  - 缺失历史字段时自动回退为 `unknown`。
- 已同步契约文档与回归：
  - `docs/result_contract_v1.md` 增补 retry 分类字段；
  - `README.md` 增加结果面 retry 可观测性说明；
  - 新增测试覆盖分类判定与结果字段输出，并更新 MCP e2e 断言。
- 已通过回归：
  - `cargo fmt && cargo test -q`（`167 + 41 + 3` tests passed）。

## T-086 V0.10-P0-SpawnAcceptedOnlyAsyncProbe (Completed 2026-03-25)

任务：将 `spawn` 收口为 accepted-only：同步路径只做 spec/request 组装并落盘，provider probe 后移到后台 worker，避免前台卡住。  
验收标准：

1. `spawn_agent` 同步路径不再执行 provider probe，调用可在 slow probe 场景快速返回。
2. `run_agent` 仍保留同步 provider 可用性校验，不改变原有拒绝语义。
3. provider 不可用时，`spawn_agent` 先返回 handle，再在后台将 run 置为 `failed` 并写入明确 unavailable 错误。
4. `run.json` 的 `probe_result` 在异步路径成功/失败都能持久化，不丢 probe 快照。
5. 新增测试覆盖“slow probe 快速返回”与“spawn accept 后异步失败”，`cargo test -q` 全量通过。
完成记录：

- 已完成执行链重构：
  - `src/mcp/server.rs::prepare_run` 去除同步 probe，仅返回 `loaded + request + execution_policy`；
  - `src/mcp/tools.rs::run_agent` 显式保留 `ensure_provider_ready`；
  - `src/mcp/tools.rs::spawn_agent` 改为 worker 内 probe，并在 unavailable 时写入失败摘要与错误信息。
- 已补运行态与持久化一致性：
  - async 成功/失败路径都会写入 `probe_result`；
  - unavailable 失败路径保留标准错误文案 `provider \`...\` is unavailable (...)` 并落盘。
- 已新增回归测试：
  - `spawn_agent_returns_before_slow_probe_completes`
  - `spawn_agent_accepts_then_fails_when_provider_unavailable`
- 已通过回归：
  - `cargo fmt`
  - `cargo test -q`（`170 + 42 + 3` tests passed）。

## T-087 V0.10-P0-RunEventsJsonlAndHeartbeat (Completed 2026-03-25)

任务：落地 run 事件流最小闭环：事件文件升级为 `events.jsonl`、`spawn` accepted/queued/probe/heartbeat 可见，并让 CLI `watch` 直接消费增量事件。  
验收标准：

1. 每个 run 目录生成 `events.jsonl`，并兼容保留 `events.ndjson`（旧读取链路不破坏）。
2. `spawn_agent` 在 accepted-only 路径写入 `run.accepted` / `run.queued` 事件。
3. 后台 worker 写入 `provider.probe.started/completed`，运行中定期写入 heartbeat，终态写入 `run.completed|run.failed|run.timed_out|run.cancelled`。
4. `watch` 在非 JSON 模式下不再只输出状态变化，而是能持续打印新增事件。
5. `timeline` 读取优先 `events.jsonl`，缺失时自动回退 `events.ndjson`。
6. `cargo test -q` 全量通过。
完成记录：

- 已完成事件持久化升级：
  - `src/mcp/persistence.rs` 事件主文件改为 `events.jsonl`；
  - 同步写入 legacy `events.ndjson`；
  - 新增 `append_run_event(...)`，支持带 `seq/ts/level/state/phase/source/message` 的增量事件行。
- 已完成 `spawn` 事件链路接入：
  - `src/mcp/tools.rs::spawn_agent` 在同步返回前写入 `run.accepted`/`run.queued`；
  - worker 侧写入 `provider.probe.started/completed`、`workspace.prepare.started`、`provider.heartbeat` 与终态 `run.*` 事件；
  - provider unavailable 的异步失败路径也写入明确失败事件。
- 已完成 CLI 消费面增强：
  - `src/main.rs` 的 `load_run_events` 改为优先 `events.jsonl`，自动回退 `events.ndjson`；
  - `watch` 改为实时打印新增事件（并保留状态行）。
- 已通过回归：
  - `cargo fmt`
  - `cargo test -q`（`170 + 42 + 3` tests passed）。

## T-088 V0.10-P0-EventsStatsWaitCliSurface (Completed 2026-03-25)

任务：把 v0.10 可观察命令面补齐到可直接使用：新增 `events/stats/wait`，并与现有 `watch/timeline` 兼容协同。  
验收标准：

1. CLI 新增 `events <handle> [--event ...] [--follow] [--interval-ms] [--timeout-secs] [--json]`。
2. CLI 新增 `stats <handle> [--json]`，输出阶段耗时、last event、stall 信号与 token usage。
3. CLI 新增 `wait <handle> [--interval-ms] [--timeout-secs] [--json]`，阻塞到终态并按状态返回退出码。
4. `events/timeline/watch` 统一优先读取 `events.jsonl`，兼容回退 `events.ndjson`。
5. 补测试覆盖：命令解析、`events.jsonl` 优先级、stats 时序计算。
6. `cargo test -q` 全量通过。
完成记录：

- 已补齐命令面：
  - `src/main.rs` 新增 `Commands::Events/Stats/Wait`；
  - 主命令分发新增 `read_events/read_stats/wait_run` 执行链；
  - `wait` 退出码映射：`succeeded=0`、`cancelled=2`、`timed_out=124`、其他失败=1。
- 已增强事件消费能力：
  - `events --follow` 支持增量输出（文本/JSON 行）；
  - `watch` 保留状态输出并实时打印新增事件；
  - `load_run_events` 统一优先读取 `events.jsonl`，缺失回退 legacy `events.ndjson`。
- 已补 stats 结果模型：
  - 新增 `RunStatsOutput` 与 `build_run_stats_output`；
  - 汇总 `queue_ms/provider_probe_ms/execution_ms/wall_ms/last_event_age_ms/stalled`；
  - 复用现有 usage 输出，保持 token 口径一致。
- 已同步文档命令面：
  - `README.md` 命令表新增 `events/stats/wait`；
  - 示例链路切换为 `events`，并标注 `timeline` 为兼容别名。
- 已新增测试：
  - `parses_events_command_flags`
  - `parses_wait_command_flags`
  - `parses_stats_command_flags`
  - `build_run_stats_output_derives_phase_and_durations_from_events`
  - `load_run_events_prefers_jsonl_when_both_formats_exist`
- 已通过回归：
  - `cargo fmt`
  - `cargo test -q`（`170 + 46 + 3` tests passed）。

## T-089 V0.10-P1-McpWatchEventsAndStatsTools (Completed 2026-03-25)

任务：在 MCP 协议面补齐 v0.10 可观察能力：新增增量事件读取与统计读取工具，供 Host 侧低成本轮询。  
验收标准：

1. MCP 新增 `watch_agent_events(handle_id, since_seq?, limit?)`，支持按 seq 增量读取。
2. MCP 新增 `get_agent_stats(handle_id)`，返回阶段耗时、last event/stalled 与 usage。
3. 事件读取优先 `events.jsonl`，缺失时回退 `events.ndjson`。
4. 现有 MCP tool 链路不回退，新增 tool 与旧 tool 可同时工作。
5. MCP roundtrip 测试覆盖新工具调用与关键字段断言。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 MCP DTO：
  - `src/mcp/dto.rs` 新增 `WatchAgentEventsInput/Output`、`RunEventOutput`、`GetAgentStatsInput/Output`。
- 已落地工具实现：
  - `src/mcp/tools.rs` 新增 `watch_agent_events`（支持 `since_seq/limit`）与 `get_agent_stats`；
  - 增加事件文件解析与 stats 计算辅助函数（queue/probe/execution/wall + stalled）。
- 已完成兼容读取策略：
  - MCP 事件读取优先 `events.jsonl`，自动回退 `events.ndjson`。
- 已补协议级回归：
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 新增工具名断言；
  - roundtrip 新增 `get_agent_stats` 与 `watch_agent_events` 调用断言。
- 已同步文档：
  - `README.md` MCP tools 列表新增 `watch_agent_events`、`get_agent_stats`。
- 已通过回归：
  - `cargo fmt`
  - `cargo test -q`（`170 + 46 + 3` tests passed）。

## T-090 V0.10-P1-StatusPsObservabilitySurface (Completed 2026-03-25)

任务：补齐状态面的人类可读可观察输出：`status/ps` 显示 phase、last event age、stalled 与 elapsed，减少“只看到 running”的黑盒感。  
验收标准：

1. CLI `status` 文本输出包含 `state/phase/last_event/last_event_age/stalled`。
2. CLI `ps` 文本输出对 running 任务展示 `phase/elapsed/last_event/stalled`。
3. MCP `get_agent_status` 输出新增可观察字段（可选，不破坏兼容）。
4. MCP `list_runs` 输出新增可观察字段（可选，不破坏兼容）。
5. MCP e2e roundtrip 覆盖新字段存在性断言。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 DTO（兼容可选字段）：
  - `src/mcp/dto.rs::AgentStatusOutput` 新增 `state/phase/last_event_at/last_event_age_ms/stalled`；
  - `src/mcp/dto.rs::RunListingOutput` 新增 `state/phase/last_event_at/last_event_age_ms/stalled/elapsed_ms`。
- 已升级 MCP 工具输出：
  - `src/mcp/tools.rs::get_agent_status` 基于事件流填充 phase/age/stalled；
  - `src/mcp/tools.rs::list_runs` 为每条 run 填充 phase/age/stalled/elapsed。
- 已升级 CLI 展示面：
  - `src/main.rs::get_status` 文本输出补充状态观测字段；
  - `src/main.rs::list_runs` 文本输出改为 `phase + elapsed + last_event + stalled` 形态；
  - 新增短时间格式化函数 `format_elapsed_short`。
- 已补协议回归：
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 增加 `get_agent_status/list_runs` 新字段断言。
- 已同步文档：
  - `README.md` 补充 `ps` 可观察字段说明。
- 已通过回归：
  - `cargo fmt`
  - `cargo test -q`（`170 + 46 + 3` tests passed）。

## T-091 V0.10-P1-BlockReasonAndLogsFollow (Completed 2026-03-25)

任务：补齐运行期阻塞原因输出与 `logs --follow`，让“运行中卡在哪里”可解释且可持续观察。  
验收标准：

1. MCP `get_agent_status/list_runs/get_agent_stats` 输出包含 `block_reason`（可选字段，兼容旧调用）。
2. CLI `status/ps/stats` 文本输出显示 `block_reason`。
3. 新增 `logs --follow`（支持 `--interval-ms/--timeout-secs`），可持续输出 runtime events 与 stdout/stderr 增量。
4. `logs --follow --json` 输出机器可读 JSON 行（event + stream 两类）。
5. README 命令面与示例更新包含 `logs --follow` 与 `block_reason` 说明。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 MCP DTO：`AgentStatusOutput/RunListingOutput/GetAgentStatsOutput` 新增 `block_reason`。
- 已在 `src/mcp/tools.rs` 落地 `block_reason` 归因：
  - 错误文本与事件启发式识别（`trust/auth/tool approval/skill discovery/workspace scan/provider unavailable/normalization/network`）；
  - stalled + phase 回退（`queueing/workspace_prepare/provider_probe/provider_boot/provider_output_wait`）。
- 已在 CLI 落地阻塞原因输出：
  - `status` 新增 `block_reason` 行；
  - `ps` 行输出新增 `block_reason`；
  - `stats` 新增 `block_reason` 字段与文本输出。
- 已新增 `logs --follow`：
  - 支持 `--follow --interval-ms --timeout-secs`；
  - 文本模式合并输出 runtime events 与 `stdout/stderr` 增量；
  - `--json` 模式输出 `kind=event|stream` 的 JSON 行。
- 已补测试：
  - `parses_logs_command_stderr_mode`（默认值校验）；
  - `parses_logs_follow_flags`；
  - `classify_block_reason_detects_provider_unavailable_from_error_text`；
  - `classify_block_reason_uses_stalled_phase_fallback`；
  - MCP roundtrip 新增 `block_reason` 字段存在性断言。
- 已同步 README：命令面新增 `logs --follow` 参数与示例，`ps` 字段说明补 `block_reason`。

## T-092 V0.10-P1-ProviderWaitSignalsAndFirstOutputWatchdog (Completed 2026-03-25)

任务：补齐 provider 启动阻塞信号事件与 first-byte watchdog，让 `events/watch/logs` 能看到“卡在哪一层”。  
验收标准：

1. 异步执行链新增 `provider.boot.started` 事件。
2. 异步执行链新增 first-byte watchdog：超过阈值无输出时写入 `provider.first_output.warning` 事件。
3. 从 provider 输出/错误文本识别并写入 `provider.waiting_for_trust/auth/tool_approval/skill_discovery/workspace_scan` 事件。
4. `block_reason` 归因逻辑支持上述新增事件（CLI + MCP 一致）。
5. README 示例补充 first-byte warning 事件跟随命令。
6. `cargo test -q` 全量通过。
完成记录：

- 已在 `src/mcp/tools.rs` 的 spawn worker 事件流增加：
  - `provider.boot.started`（进入 provider 启动阶段）；
  - `provider.first_output.warning`（默认 8s 无输出触发一次）。
- 已新增 provider wait signal 识别：
  - 文本命中后落盘 `provider.waiting_for_trust/auth/tool_approval/skill_discovery/workspace_scan` 事件；
  - 识别来源覆盖 dispatch `stdout/stderr` 与错误路径 `error_message`。
- 已更新 `block_reason` 规则：
  - MCP 与 CLI 都支持从新增 wait 事件和 first-output warning 直接归因。
- 已补测试：
  - `src/mcp/tools.rs` 新增 wait signal / first-output warning 归因单测；
  - `src/main.rs` 新增 wait event 归因单测。
- 已同步 README 示例：
  - 新增 `events --event provider.first_output.warning --follow`。

## T-093 V0.10-P1-StatsPhaseSplitsAndWaitSummary (Completed 2026-03-25)

任务：增强 `stats` 结果面，补齐 phase 细分耗时和 wait 信号汇总，降低“只有总耗时”的排障成本。  
验收标准：

1. `get_agent_stats`（MCP）新增：`workspace_prepare_ms`、`provider_boot_ms`、`first_output_warned`、`first_output_warning_at`、`current_wait_reason`、`wait_reasons`。
2. CLI `stats` 文本输出展示上述新增字段。
3. CLI `build_run_stats_output` 与 MCP `build_agent_stats_output` 都基于事件流计算新增字段，口径一致。
4. MCP roundtrip 测试新增 stats 字段存在性断言。
5. README 说明补充 stats 新字段能力。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 stats 数据模型：
  - `src/mcp/dto.rs::GetAgentStatsOutput` 新增 phase split 与 wait summary 字段。
- 已升级 MCP stats 计算：
  - `src/mcp/tools.rs` 新增 `workspace_prepare_ms/provider_boot_ms` 计算；
  - 新增 `first_output_warned/first_output_warning_at`；
  - 新增 `wait_reasons/current_wait_reason` 汇总（由 `provider.waiting_for_*` 事件派生）。
- 已升级 CLI stats：
  - `src/main.rs::RunStatsOutput` 同步新增字段；
  - `read_stats` 文本输出新增字段打印；
  - `build_run_stats_output` 计算口径与 MCP 对齐。
- 已补测试：
  - `build_run_stats_output_derives_phase_and_durations_from_events` 覆盖新增时序字段；
  - `mcp_transport_roundtrip_for_all_tools` 新增 stats 字段断言；
  - 保留并通过 wait reason/block reason 相关测试。
- 已同步 README：
  - 新增 stats 能力说明（phase splits + first-output watchdog + wait reasons）。

## T-094 V0.10-P1-PhaseProgressViewForFollowCommands (Completed 2026-03-25)

任务：为 `watch/events/logs` 的 follow 视图补齐 phase 聚合进度行，降低“事件很多但看不出整体进展”的认知成本。  
验收标准：

1. 新增 phase 聚合函数，按事件序列计算各 phase 累计时长并标记当前 phase。
2. `watch` 文本模式在 follow 循环中输出滚动 `phase_progress` 行（变化时输出）。
3. `events --follow` 文本模式输出滚动 `phase_progress` 行（变化时输出）。
4. `logs --follow` 文本模式输出滚动 `phase_progress` 行（变化时输出）。
5. JSON 模式行为保持兼容（不插入额外文本污染 JSON 行）。
6. README 说明补充 phase progress 行为。
7. `cargo test -q` 全量通过。
完成记录：

- 已在 `src/main.rs` 新增 phase 聚合能力：
  - `build_phase_progress_line(events, terminal, now)` 统一生成进度摘要；
  - 输出格式：`phase_progress: <phase=duration ... current*=...> wall=<...>`。
- 已接入 follow 命令：
  - `watch`、`events --follow`、`logs --follow` 都在文本模式下输出 `phase_progress`；
  - 仅在进度行变化时输出，避免刷屏。
- JSON 兼容保持：
  - `events --follow --json` 与 `logs --follow --json` 继续仅输出 JSON 行。
- 已补测试：
  - `build_phase_progress_line_marks_current_phase`；
  - `build_phase_progress_line_terminal_has_no_current_marker`。
- 已同步 README：
  - 增加 `watch/events/logs --follow` phase_progress 说明。

## T-095 V0.10-P1-PhaseFilterAndPhaseTimeout (Completed 2026-03-25)

任务：为 follow 观察命令补充阶段过滤与阶段超时，提升“只盯某阶段排障”的效率。  
验收标准：

1. `watch/events/logs` 新增 `--phase <name>` 过滤参数（事件输出按 phase 过滤）。
2. `watch/events/logs` 新增 `--phase-timeout-secs <n>`（阶段长期不变化时超时退出）。
3. `events` 非 follow 模式也支持 `--phase` 过滤。
4. `phase_progress` 与 `--phase` 联动（非目标 phase 时不输出 progress 行）。
5. JSON 模式兼容不破坏：`events/logs --json` 仍只输出 JSON。
6. 命令解析与 progress/filter 行为有测试覆盖。
7. `cargo test -q` 全量通过。
完成记录：

- 已扩展 CLI 命令面：
  - `Commands::Watch/Events/Logs` 新增 `phase` 与 `phase_timeout_secs`；
  - `read_events/read_logs/watch_run` 执行链同步接入。
- 已实现 phase 过滤：
  - `filter_timeline_events` 新增 phase 过滤参数；
  - `events` follow 与非 follow 均可按 phase 过滤事件输出；
  - `watch/logs --follow` 文本事件输出按 phase 过滤。
- 已实现 phase timeout：
  - 在 `watch/events/logs --follow` 循环维护 `observed_phase + observed_phase_started_at`；
  - 超过 `--phase-timeout-secs` 后返回 `124` 并输出超时原因。
- 已升级 phase progress：
  - `build_phase_progress_line` 新增 `phase_filter` 参数；
  - phase filter 不匹配时不输出 progress 行，避免噪音。
- 已补测试：
  - `parses_logs_command_stderr_mode/parses_logs_follow_flags`（新增 phase/phase-timeout 断言）；
  - `parses_events_command_flags`（新增 phase/phase-timeout 断言）；
  - `parses_watch_command_flags/parses_watch_phase_timeout_flags`；
  - `build_phase_progress_line_*` 增加 phase filter 场景。
- 已同步 README：
  - 命令面新增 `--phase` / `--phase-timeout-secs`；
  - 增加 phase-timeout 使用说明。

## T-096 V0.10-P1-McpPhaseFilterAndWatchdog (Completed 2026-03-25)

任务：将 phase 过滤与 phase watchdog 能力对齐到 MCP 事件工具面，避免 Host 侧只能自行拼接状态逻辑。  
验收标准：

1. MCP `watch_agent_events` 入参新增 `phase` 与 `phase_timeout_secs`。
2. MCP `watch_agent_events` 出参新增 `current_phase`、`current_phase_age_ms`、`phase_timeout_hit`。
3. `watch_agent_events` 支持按 phase 过滤事件返回。
4. `phase_timeout_hit` 基于“当前 phase 持续时长”计算，可被 Host 直接消费。
5. MCP roundtrip 测试覆盖新增字段存在性与 phase 过滤调用路径。
6. README MCP 工具说明更新。
7. `cargo test -q` 全量通过。
完成记录：

- 已扩展 DTO：
  - `src/mcp/dto.rs::WatchAgentEventsInput` 新增 `phase/phase_timeout_secs`；
  - `src/mcp/dto.rs::WatchAgentEventsOutput` 新增 `current_phase/current_phase_age_ms/phase_timeout_hit`。
- 已升级工具实现：
  - `src/mcp/tools.rs::watch_agent_events` 支持 phase 过滤；
  - 新增 `current_phase_age_ms` 计算；
  - 新增 phase timeout 命中计算（`phase_timeout_hit`）。
- 已补单测：
  - `current_phase_age_ms_tracks_latest_phase_window`。
- 已补 MCP roundtrip：
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 使用 `phase + phase_timeout_secs` 调用并断言新增输出字段。
- 已同步 README：
  - MCP tools 段新增 `watch_agent_events` phase/watchdog 能力说明。

## T-097 V0.10-P1-McpWatchRunPhaseWatchdog (Completed 2026-03-25)

任务：将 phase watchdog 能力从 `watch_agent_events` 扩展到 `watch_run`，减少 Host 端双工具拼装复杂度。  
验收标准：

1. `WatchRunInput` 新增 `phase`、`phase_timeout_secs`。
2. `WatchRunOutput` 新增 `current_phase`、`current_phase_age_ms`、`phase_timeout_hit`。
3. `watch_run` 在轮询过程中计算当前 phase 持续时长，并支持 phase timeout 命中返回。
4. 终态返回仍保持兼容（`terminal=true`、`timed_out=false`），并带上 phase 观测字段。
5. MCP roundtrip 覆盖 `watch_run` 新参数调用与新增字段断言。
6. README MCP tools 说明同步更新。
7. `cargo test -q` 全量通过。
完成记录：

- 已扩展 DTO：
  - `src/mcp/dto.rs::WatchRunInput` 新增 `phase/phase_timeout_secs`；
  - `src/mcp/dto.rs::WatchRunOutput` 新增 `current_phase/current_phase_age_ms/phase_timeout_hit`。
- 已升级 `watch_run`：
  - `src/mcp/tools.rs::watch_run` 每轮读取事件并计算当前 phase age；
  - 支持 phase scoped timeout，命中后返回 `timed_out=true` 与 `phase_timeout_hit=true`；
  - 普通 timeout/终态路径均返回 phase 观测字段。
- 已补 MCP roundtrip：
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 对 `watch_run` 传入 `phase + phase_timeout_secs`；
  - 新增 `current_phase/current_phase_age_ms/phase_timeout_hit` 字段断言。
- 已同步 README：
  - MCP tools 段补充 `watch_run` phase/watchdog 能力说明。

## T-098 V0.10-P1-WatchAdviceSurface (Completed 2026-03-25)

任务：为 MCP watch 工具补齐统一 `advice` 输出，降低 Host 端“拿到状态但不知道下一步做什么”的集成成本。  
验收标准：

1. `WatchRunOutput` 与 `WatchAgentEventsOutput` 新增 `block_reason` 与 `advice` 字段。
2. `watch_run/watch_agent_events` 统一生成建议：
   - phase timeout 命中时给出阶段排障建议；
   - 常见阻塞原因（trust/auth/approval/workspace/skills/network/provider unavailable）给出操作建议；
   - 终态给出下一步动作建议（如 `get_run_result` / 查 `stderr`）。
3. MCP roundtrip 断言新增字段存在。
4. README MCP 工具说明更新到 `advice`。
5. 新增单测覆盖建议生成逻辑。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 DTO：
  - `src/mcp/dto.rs::WatchRunOutput` 新增 `block_reason/advice`；
  - `src/mcp/dto.rs::WatchAgentEventsOutput` 新增 `block_reason/advice`。
- 已升级工具实现：
  - `src/mcp/tools.rs` 新增 `build_watch_advice`；
  - `watch_run/watch_agent_events` 接入 `build_event_runtime_snapshot` 的 `block_reason`；
  - 输出 `advice` 支持 phase-timeout + reason + terminal next step 组合。
- 已补测试：
  - `build_watch_advice_includes_timeout_and_reason_guidance`；
  - `build_watch_advice_includes_terminal_next_step`；
  - MCP roundtrip 新增 `watch_run/watch_agent_events` 的 `block_reason/advice` 字段断言。
- 已同步 README：
  - `watch_run/watch_agent_events` 输出说明补充 `block_reason/advice`。

## T-099 V0.10-P1-StatusStatsAdviceSurface (Completed 2026-03-25)

任务：把 `advice` 能力从 MCP watch 工具扩展到 `get_agent_status/get_agent_stats`，让仅轮询状态的 Host 也能拿到下一步建议。  
验收标准：

1. `AgentStatusOutput` 与 `GetAgentStatsOutput` 新增 `advice` 字段（默认空数组）。
2. `get_agent_status/get_agent_stats` 复用统一建议生成逻辑（基于 status/phase/block_reason）。
3. 新字段保持协议兼容（`serde(default)`，旧客户端可忽略）。
4. MCP roundtrip 测试补齐 `get_agent_status/get_agent_stats` 的 `advice` 字段断言。
5. README MCP 工具说明同步到 polling 能力。
6. `cargo test -q` 全量通过。
完成记录：

- 已扩展 DTO：
  - `src/mcp/dto.rs::AgentStatusOutput` 新增 `advice`；
  - `src/mcp/dto.rs::GetAgentStatsOutput` 新增 `advice`；
  - 两处均使用 `#[serde(default)]` 保持兼容。
- 已升级工具实现：
  - `src/mcp/tools.rs::get_agent_status` 接入 `build_watch_advice`；
  - `src/mcp/tools.rs::build_agent_stats_output` 接入 `build_watch_advice`。
- 已补 MCP roundtrip：
  - `src/mcp/server.rs::mcp_transport_roundtrip_for_all_tools` 新增 `status_after_done` 与 `stats` 的 `advice` 字段断言。
- 已同步 README：
  - MCP 工具说明补充 `get_agent_status/get_agent_stats` 返回 `block_reason/advice`。

## T-100 V0.10-P1-GeminiResearchStableScratchWorkspace (Completed 2026-03-25)

任务：把 Gemini 简单 research 任务从“默认继承当前仓库 working_dir”改为“默认长期复用 stable scratch workspace”，降低 trust/skills/discovery 噪音。  
验收标准：

1. `working_dir_policy=auto` 下，满足以下条件时默认切到 stable scratch workspace：
   - provider=`gemini`
   - sandbox=`read_only`
   - delegation_context=`minimal`
   - 无 `selected_files`、无 `plan_ref`
   - 任务有 research 信号（`stage=research|plan` 或 agent tag 含 `research`）
2. scratch 路径长期复用，默认落到 `~/.mcp-subagent/provider-workspaces/gemini/research`，并支持环境变量覆盖。
3. run metadata `workspace.mode` 能区分该路径（新增 `stable_scratch`）。
4. cleanup 不会删除 stable scratch 目录。
5. 新增单测覆盖：scratch 路由命中、selected_files 保护分支、scratch 路径解析、cleanup 语义。
6. README 补充默认行为与覆盖方式说明。
7. `cargo test -q` 全量通过。
完成记录：

- 已升级 `src/runtime/workspace.rs`：
  - 新增 `WorkspaceMode::StableScratch`；
  - 新增 Gemini research-only 自动路由判定；
  - 新增 stable scratch 路径解析逻辑（默认 HOME 下路径，支持 `MCP_SUBAGENT_GEMINI_RESEARCH_SCRATCH_DIR` 覆盖）。
- 已升级 `src/runtime/cleanup.rs`：
  - `StableScratch` 与 `InPlace` 一样不创建 cleanup guard，不做目录删除。
- 已升级 `src/mcp/service.rs`：
  - workspace metadata 映射新增 `stable_scratch`。
- 已补测试：
  - `auto_policy_routes_gemini_research_profile_to_stable_scratch`
  - `auto_policy_keeps_in_place_when_gemini_research_has_selected_files`
  - `resolve_stable_gemini_scratch_dir_uses_home_when_unset`
  - `stable_scratch_workspace_has_no_cleanup_guard`
- 已同步 README：
  - 配置环境变量段新增 `MCP_SUBAGENT_GEMINI_RESEARCH_SCRATCH_DIR`；
  - 推荐命令流补充 stable scratch 默认行为说明。

## T-101 V0.10-P1-GeminiStableScratchDiscoveryOverride (Completed 2026-03-25)

任务：解决 stable scratch 命中后 Gemini 仍因 `native_discovery=isolated` 触发 auth fallback 慢启动的问题，在运行时自动降级 discovery 策略。  
验收标准：

1. 当 workspace mode=`stable_scratch` 且 provider=`gemini` 且 `native_discovery=isolated` 时，运行时自动改为 `minimal`。
2. override 仅作用于本次执行，不改写 agent 文件。
3. workspace notes 记录 override 原因，便于 run 审计。
4. 新增单测覆盖 override 触发与 notes 写入。
5. README 补充 stable scratch + discovery override 行为说明。
6. `cargo test -q` 全量通过。
完成记录：

- 已升级 `src/mcp/service.rs`：
  - 新增 `apply_workspace_runtime_overrides`；
  - `run_dispatch` 改为基于 `effective_spec` 执行（memory resolve + dispatcher + runner 统一吃 override 后 spec）；
  - 命中 stable scratch 时把 Gemini `native_discovery` 从 `isolated` 降级到 `minimal`，并写入 workspace note。
- 已补测试：
  - `stable_scratch_overrides_gemini_isolated_discovery_to_minimal`。
- 已同步 README：
  - stable scratch 段补充“自动降级 isolated->minimal 避免 auth/trust fallback loops”说明。

## T-102 V0.10-P1-GlobalEventsFollow (Completed 2026-03-25)

任务：补齐全局事件流命令面，让排障时不必先拿单个 handle，再能直接观察所有 run 的事件推进。  
验收标准：

1. `events` 子命令支持 `--all`，形成命令面：`events [<handle-id>] [--all] ...`。
2. `events --all`（非 follow）可聚合输出所有 run 的事件（支持 `--event/--phase` 过滤）。
3. `events --all --follow` 支持跨 run 增量输出（含 handle 前缀），并支持 `--timeout-secs/--phase-timeout-secs`。
4. 单 handle 行为保持兼容（原有 `events <handle-id>` 路径不变）。
5. CLI 解析测试覆盖 `events --all` 参数组合。
6. 新增聚合快照测试覆盖多 handle 事件加载与过滤。
7. README 命令面和示例补充 `events --all --follow`。
8. `cargo test -q` 全量通过。
完成记录：

- 已升级 `src/main.rs`：
  - `Commands::Events` 新增 `all: bool`，`handle_id` 改为可选并与 `--all` 互斥；
  - `read_events` 新增 all 分支校验与分流；
  - 新增 `collect_run_event_snapshots` 与 `read_events_all`，实现全局一次性/跟随模式；
  - follow 全局模式支持 `--event/--phase` 过滤、全局 timeout、phase-timeout。
- 已补测试：
  - `parses_events_all_command_flags`；
  - `collect_run_event_snapshots_loads_all_handles_and_filters`。
- 已同步 README：
  - command surface 改为 `events [<handle-id>] [--all] ...`；
  - 示例新增 `mcp-subagent events --all --follow`。

## T-103 V0.10-P1-GlobalEventsContinuousFollow (Completed 2026-03-25)

任务：修复 `events --all --follow` 的“非连续流”行为，避免在当前活跃 run 结束后自动退出导致看起来像一次性拉取。  
验收标准：

1. `events --all --follow` 默认持续监听（不因当前 active run 归零自动退出）。
2. 仅在显式超时（`--timeout-secs` / `--phase-timeout-secs`）或用户中断时退出。
3. 单 handle `events <handle-id> --follow` 语义保持不变。
4. README 明确 `events --all --follow` 是 continuous stream 模式。
5. `cargo test -q` 全量通过。
完成记录：

- 已修复 `src/main.rs::read_events_all`：
  - 移除“active runs 清空即退出”的逻辑；
  - 全局 follow 改为持续轮询并输出增量事件。
- 已同步 README：
  - 在示例段明确 `events --all --follow` 会持续监听直到 Ctrl-C 或 timeout。

## T-104 V0.10-P1-GlobalEventsNoiseAndAuthFalsePositiveFix (Completed 2026-03-25)

任务：修复全局事件流的两类可用性问题：`phase_progress` 噪音刷屏、以及成功任务误判 `auth_required`。  
验收标准：

1. `events --all --follow` 的 `phase_progress` 只在对应 handle 有新增事件时更新，不再按轮询周期对旧 run 刷屏。
2. `auth_required` 检测不再把“已加载凭证”日志误判为阻塞（如 `Loaded cached credentials` / keychain fallback）。
3. `succeeded` 终态不再输出 `block_reason=auth_required` 这类误导阻塞原因。
4. provider heartbeat 在首输出前保持 `provider_boot` phase，避免早期误切到 `running` 造成 phase 抖动。
5. 新增单测覆盖 cached credential 误判防护与 succeeded block_reason 行为。
6. `cargo test -q` 全量通过。
完成记录：

- 已升级 `src/main.rs`：
  - 新增 `auth_is_ready_signal/auth_is_wait_signal`；
  - `classify_block_reason_from_text` 收紧 auth 识别；
  - `classify_block_reason` 对 `status=succeeded` 直接返回 `None`；
  - `read_events_all` 改为仅在 handle 有新增事件时更新该 handle 的 `phase_progress`。
- 已升级 `src/mcp/tools.rs`：
  - 新增同等 auth 信号判定函数，`detect_provider_wait_signal` 与 `classify_block_reason_from_text` 共用；
  - `classify_block_reason` 对 `RunStatus::Succeeded` 返回 `None`；
  - dispatch 心跳事件 phase 从 `running` 调整为 `provider_boot`（首输出前阶段语义更准确）。
- 已补测试：
  - main: `classify_block_reason_ignores_cached_credentials_text`、`classify_block_reason_is_none_for_succeeded_status`
  - mcp/tools: `detect_provider_wait_signal_ignores_cached_credentials_log`、`classify_block_reason_is_none_for_succeeded_status`、`classify_block_reason_from_text_ignores_cached_credentials_log`

## T-105 V0.10-P1-SpawnAcceptedEnvelopeAndCoreEventCoverage (Completed 2026-03-25)

任务：收口 `spawn` accepted 返回语义，并补齐 v0.10 必备事件里的关键缺口（context/parse/cancel/output delta）。  
验收标准：

1. `spawn_agent` 输出包含 accepted envelope：`status/state/phase/queued_at`（且 `status=accepted`）。
2. 运行成功路径事件流补齐：`workspace.prepare.completed`、`context.compile.started/completed`、`parse.started/completed`。
3. provider 输出事件补齐：`provider.stdout.delta` 与 `provider.stderr.delta`（至少包含 bytes/lines 指标）。
4. `cancel_agent` 会写入 `run.cancelled` 事件，Host 可直接观察取消终态事件。
5. CLI/README 与测试同步更新，`cargo test -q` 全量通过。
完成记录：

- 已扩展 `src/mcp/dto.rs::SpawnAgentOutput`：新增 `state/phase/queued_at` 字段，并保持 `status` 兼容。
- 已升级 `src/mcp/tools.rs::spawn_agent`：
  - 返回 accepted envelope（`status/state/phase=accepted` + `queued_at`）；
  - 新增 `append_transition_derived_events`，从 `status_history` 衍生补齐 `workspace.prepare.completed`、`context.compile.*`、`parse.*`；
  - 新增 `append_provider_output_delta_events`，输出 `provider.stdout.delta` / `provider.stderr.delta`（bytes/lines）。
- 已升级 `src/mcp/tools.rs::cancel_agent`：取消路径追加 `run.cancelled` 事件。
- 已同步 `src/main.rs` 非 JSON spawn 输出，补充 `state/phase/queued_at` 展示。
- 已同步 MCP/集成测试与工具单测：
  - `src/mcp/server.rs` 断言 spawn accepted envelope；
  - roundtrip 断言取消后可观测 `run.cancelled`；
  - `src/mcp/tools.rs` 新增 transition-derived events 与 provider delta events 单测。
- 已同步 `README.md`：说明 `spawn/submit --json` 的 accepted envelope 字段。
- 已通过 `cargo fmt && cargo test -q`（186 + 58 + 3 全通过）。

## T-106 V0.10-P1-IncrementalFollowEventCursor (Completed 2026-03-25)

任务：将 `events --follow` 从“每轮全量重读 events 文件”改为“基于文件 offset 的增量消费”，降低轮询开销并让事件流语义更接近实时 tail。  
验收标准：

1. `events <handle-id> --follow` 使用增量 cursor，只消费新增事件，不重复解析历史全量文件。
2. `events --all --follow` 同样使用 per-handle 增量 cursor，不再每轮扫描全量历史事件。
3. 保持现有行为兼容：首次进入 follow 仍可看到已有历史事件，过滤参数 `--event/--phase` 继续生效。
4. phase timeout / global timeout / phase_progress 输出语义保持不变。
5. 新增单测覆盖增量读取和 partial line 场景。
6. `cargo test -q` 全量通过。
完成记录：

- 已在 `src/main.rs` 增加增量读取基础设施：
  - `EventStreamCursor`、`FollowEventState`
  - `resolve_events_file_path`
  - `parse_timeline_event_line`
  - `load_run_events_incremental`（offset + trailing partial line 处理）
- 已改造 `read_events`（单 handle follow）：
  - 用 cursor + in-memory accumulated events 替代每轮 `load_run_events` 全量读取。
- 已改造 `read_events_all`（全局 follow）：
  - 用 per-handle `FollowEventState` 增量消费事件；
  - 保留 `--event/--phase` 过滤、phase timeout 和 `phase_progress` 输出逻辑。
- 已新增测试：
  - `load_run_events_incremental_only_returns_appended_events`
  - `load_run_events_incremental_handles_partial_trailing_line`
- 已通过 `cargo fmt && cargo test -q`（186 + 60 + 3 全通过）。

## T-107 V0.10-P1-SyntheticEventProgressNoiseFix (Completed 2026-03-25)

任务：修复你实测日志里 `phase_progress` 被 synthetic 衍生事件污染导致尾部出现多个 `0ms` phase 段的问题。  
验收标准：

1. 衍生事件（`workspace.prepare.completed/context.compile*/parse*`）保留事件可观测性，但显式标记 synthetic。
2. `phase_progress` 计算忽略 synthetic 事件，不再出现末尾 `context_compile/parse=0ms` 抖动。
3. 新增单测覆盖 synthetic 事件不会进入 phase_progress 分段。
4. `cargo test -q` 全量通过。
完成记录：

- 已升级 `src/mcp/tools.rs::append_transition_derived_events`：
  - 为衍生事件 detail 增加 `{ synthetic: true, derived_from: "status_history" }`。
- 已升级 `src/main.rs`：
  - 新增 `is_synthetic_progress_event`；
  - `build_phase_progress_line` 跳过 synthetic 事件。
- 已新增测试：
  - `build_phase_progress_line_ignores_synthetic_events`。
- 已通过 `cargo fmt && cargo test -q`（186 + 61 + 3 全通过）。

## T-108 V0.10-P1-CliSpawnAcceptedOnlyNoWait (Completed 2026-03-25)

任务：移除 CLI `spawn/submit` 的 `wait_for_run` 阻塞，让命令行也符合 accepted-only 语义。  
验收标准：

1. `mcp-subagent spawn ... --json` 返回后进程立即退出，不等待 run 完成。
2. 输出保持 accepted envelope（`handle_id/status/state/phase/queued_at`）。
3. `submit` 与 `spawn` 行为保持一致。
4. README 命令行为说明同步更新。
5. 新增测试覆盖 CLI spawn 不等待（最少覆盖逻辑级行为）。
6. `cargo test -q` 全量通过。
完成记录：

- 已升级 `src/main.rs::spawn_agent`：
  - 去除默认 `wait_for_run` 阻塞；
  - 以 `cli_spawn_waits_for_completion()` 统一控制（当前固定为 `false`），CLI `spawn/submit` 立即返回 accepted 结果。
- 已新增测试：
  - `cli_spawn_does_not_wait_for_completion`（逻辑级行为锁定）。
- 已同步 `README.md`：
  - 补充说明 CLI `spawn/submit` accepted 后立即退出，后续用 `watch/events/stats/result` 观察。
- 已通过 `cargo fmt && cargo test -q`（186 + 62 + 3 全通过）。

## T-109 V0.10-P1-RealtimeContextParseWorkspaceEvents (Pending)

任务：将 `workspace/context/parse` 事件从 synthetic 尾部补写改为运行时实时事件。  
验收标准：

1. `workspace.prepare.completed` 在 workspace 准备完成时即时写入。
2. `context.compile.started/completed` 在编译前后即时写入。
3. `parse.started/completed` 在 summary 解析前后即时写入。
4. 移除相应 synthetic 补写路径，不再依赖 `status_history` 回填。
5. 新增测试覆盖事件时间顺序（probe -> workspace -> context -> parse -> completed）。
6. `cargo test -q` 全量通过。

## T-110 V0.10-P1-ProviderDeltaStreamingRuntimePath (Pending)

任务：把 `provider.stdout.delta/provider.stderr.delta` 从结束后一次性写入改成运行期流式增量写入。  
验收标准：

1. provider 执行期间可持续看到 stdout/stderr delta 事件，不等 run 结束。
2. `provider.first_output` 保留并在首次真实输出时触发。
3. 失败/取消路径下也不丢已产生的 delta 事件。
4. `watch/events/logs` 能在任务进行中消费到这些 delta 事件。
5. 新增测试覆盖至少一个 provider runner 的增量输出路径（可用 fake runner fixture）。
6. `cargo test -q` 全量通过。

## T-111 V0.10-P1-WatchIncrementalCursorParity (Pending)

任务：将 `watch` 路径与 `events --follow` 对齐为增量 cursor 消费，消除全量轮询读取。  
验收标准：

1. `watch <handle> --follow`（默认）不再每轮全量 `load_run_events`。
2. 保持 phase_progress、phase_timeout、terminal 退出语义不变。
3. 与 `events --follow` 输出一致性保持（同一 run 同阶段不产生自相矛盾）。
4. 新增测试覆盖 `watch` 增量读取与超时行为。
5. `cargo test -q` 全量通过。

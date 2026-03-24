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

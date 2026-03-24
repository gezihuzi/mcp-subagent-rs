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

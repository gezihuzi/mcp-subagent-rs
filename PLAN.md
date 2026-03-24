# PLAN.md

## North Star

在不透传父会话 raw transcript 的前提下，先交付可验证的 runtime 内核：统一 AgentSpec、上下文编译与结构化 Summary 解析，为后续 MCP server 与 provider runner 打地基。

## Milestones

### Phase 0 - Core Foundation (current)

目标：先把“规范、校验、上下文、摘要”做硬，保证后续 runner/mcp 接入不会返工。
依赖顺序：Spec -> Validation -> ContextCompiler -> Summary Parser -> Tests。
回滚策略：所有新增功能均在独立模块，不改动运行时外部接口；必要时可按模块回退。

### Phase 1 - Runtime Skeleton

目标：引入 dispatcher 状态流和 mock runner，打通 run 请求到结构化结果的闭环。
依赖顺序：Phase 0 完成后推进。
风险：状态机与错误模型不稳定。
回滚策略：保留 mock-only 路径，不引入真实 provider 副作用。

### Phase 2 - MCP Surface + Provider Probe

目标：接入 rmcp stdio 最小 server、list_agents/run_agent，接入 provider probe。
依赖顺序：Phase 1 完成后推进。
风险：rmcp API 变动、CLI 探测兼容性。
回滚策略：feature-gate MCP 与 probe，保留本地 CLI 自测入口。

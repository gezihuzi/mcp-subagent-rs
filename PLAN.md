# PLAN.md

## North Star

按 `docs/mcp-subagent_tech_design_v0.10.md` 推进“可观察、可解释、可中断”的本地多 LLM 子代理 runtime：`spawn` 先接受、执行后置，任务阶段与阻塞原因可见，结果 native-first、统计可追踪。

## Execution Strategy (v1.1 Experience Shell Current)

### Batch V1.1-P0 - Sub Entry + Profile Dispatch（已完成）

目标：先把“无切换入口”做出来，新增极简 `sub <profile> <task>` 入口与 profile 映射，默认以交互态流式输出运行，保留现有 `mcp-subagent` 全量命令面不变。
依赖顺序：`T-166(Completed) -> T-167`。
回滚策略：仅新增体验层入口与配置读取，不改动现有 `run/spawn/submit/watch` 主路径；若入口设计不达预期，可单独回退 `sub` 与 profile 映射，不影响 runtime 核心能力。
风险与控制：若引入第二套配置会导致配置分裂；通过扩展现有 `config.toml` 的 `[profiles.*]` 命名空间收口，避免新增平行配置文件。

### Batch V1.1-P1 - Unified Permission Broker + Direct Workspace（已完成）

目标：补齐统一权限模型，让跨目录读写行为具备“可申请、可批准、可恢复”的一致语义，同时新增 `working_dir_policy=direct` 以支持实时写回源目录。
依赖顺序：`T-167 -> T-168`。
回滚策略：`direct` 作为显式 opt-in 策略，不改变现有 `auto/git_worktree/temp_copy` 默认行为；权限 broker 先以事件和状态扩展接入，不改现有成功路径。
风险与控制：直接写源目录会提高误改风险；通过 profile 显式启用、保留 `serialize` 冲突策略、并在 `permission.requested` 阶段强制用户确认控制风险。

### Batch V1.1-P2 - Rescue Render Adapter + MCP Alias（已完成）

目标：将 Codex 风格输出（P1/P2、`Update(path)`、apply 提示）放到独立 render adapter，并补 `codex` MCP 入口，做到“体验接近官方、契约保持稳定”。
依赖顺序：`T-168 -> T-169`。
回滚策略：仅新增 adapter/render 与 alias，不修改 `mcp-subagent.result.v1` 主契约；若渲染效果不稳定，可关闭 profile 级渲染开关回退到默认 summary。
风险与控制：若把渲染逻辑侵入 summary contract 会污染通用 runtime；通过严格分层在 adapter 层完成格式化，确保多 provider 兼容性。

### Batch V1.2-P0 - Permission Decision + Resume（已完成）

目标：把 `permission.requested` 从“直接失败”收口为“等待用户决策后可继续”的闭环语义，补齐 `approve/deny` 工具与同 handle 续跑能力，解决 direct workspace 权限阻塞下的无感续跑体验。
依赖顺序：`T-169 -> T-170`。
回滚策略：保持既有 run artifact/result 契约不变，仅扩展 runtime state（pending permission context）与 MCP tool 面；若续跑语义不稳定，可回退 `approve/deny` 工具并恢复旧失败路径。
风险与控制：权限等待状态若不清理会造成伪阻塞；通过 `pending_permission_runs` 生命周期管理、`permission.approved/permission.denied` 事件、以及 cancel/terminal 分支统一清理避免悬挂状态。

### Batch V1.2-P1 - CLI Permission Controls + Event Schema + Smoke（已完成）

目标：把权限决策闭环从 MCP 内部能力扩展到 CLI 体验层，新增 `approve/deny` 入口，统一 `permission.requested` 事件 detail 结构（枚举化关键字段），并补齐 direct workspace approve 流程端到端 smoke。
依赖顺序：`T-170 -> T-171`。
回滚策略：仅扩展 CLI 子命令、权限事件 detail 构造与增量 smoke，不修改 `mcp-subagent.result.v1` 契约；若 CLI 决策入口体验不达预期，可单独回退 `approve/deny` 命令与 smoke 脚本，不影响既有 MCP 工具链。
风险与控制：CLI 进程边界可能导致 pending context 丢失和续跑中断；通过 `approve/deny` 先加载持久化 run、缺失 pending 时按 run snapshot 重建上下文、以及 CLI 默认 keepalive 到任务落盘，确保跨进程也能稳定续跑。

## Execution Strategy (v0.10 Current)

### Batch V1.0-P11 - Release Branch Cut（已完成）

目标：在 `develop` 已具备 `v0.10.0` release cut point 且工作树干净的前提下，用 `git flow release start 0.10.0` 正式切出 release 分支，冻结后续收口位置，不再继续在 `develop` 上混入发布准备变更。
依赖顺序：`T-165(Completed)`。
回滚策略：仅执行 git-flow release branch cut，并回填计划/任务记录；若 branch naming 或切版版本号判断有误，可删除错误 release branch 后重新从 `develop` 切出，不影响已完成的 v0.10 内容提交。
风险与控制：若在 dirty worktree 上直接起 release，git-flow 会拒绝或把未提交改动混进 release 准备链；通过先把 `T-164` 提交到 `develop`、确认 worktree clean，再执行 `git flow release start 0.10.0`，控制分支起点的可追溯性。

### Batch V1.0-P10 - Release Cut Automation（已完成）

目标：把 `v0.10.0` 现有的 release checklist 收成一条可重复执行的自动化命令，避免后续切版继续依赖人肉逐项核对 `Cargo.toml`、`Cargo.lock`、`PRESET_CATALOG_VERSION`、`CHANGELOG.md`、release doc 和验证命令。
依赖顺序：`T-164(Completed)`。
回滚策略：仅新增 release check 脚本并更新 release 文档引用，不改 runtime、CLI 或 state layout；若脚本策略过严，可单独回退脚本与文档而不影响已完成的 v0.10 功能闭环。
风险与控制：shell 校验若写死过多路径或格式，后续小版本 cut 可能频繁误报；通过把版本号做成显式参数，并只校验当前 release contract 必需位点与既有验证命令，控制维护成本。

### Batch V1.0-P9 - Release Prep（已完成）

目标：把当前已完成的 v0.10 runtime/bridge/CLI 收口正式打成可发布版本，统一 `Cargo.toml`、preset catalog version、`CHANGELOG.md`、release checklist 文档与测试断言，形成明确的 `v0.10.0` cut point。
依赖顺序：`T-163(Completed)`。
回滚策略：仅修改版本位点、release 文档、changelog 与相关断言，不改 runtime 行为；若发现版本命名需要调整，可单独回退 release surfaces，而不影响当前功能闭环。
风险与控制：版本位点若不同步，会导致 generated-root manifest、doctor drift 诊断与 release 文档各自报不同版本；通过一次性同步 `Cargo.toml`、`PRESET_CATALOG_VERSION`、CHANGELOG、release doc 和测试断言，避免发布后再补丁修文档。

### Batch V1.0-P8 - Bridge Contract Freeze（已完成）

目标：冻结 `generated root / project bridge / bridge-only repair` 这条外部契约，统一 README、generated README、`doctor` advice/issue 文案、`init` notes/error wording，并补一条面向发布的端到端回归，把 `drift -> refresh-bootstrap -> sync-project-config-only -> lexical cwd` 串成单条可复验故事。
依赖顺序：`T-162(Completed)`。
回滚策略：仅收口用户可见文案、修复建议与 smoke，不改 runtime/bridge 主逻辑；若文案收紧引发兼容顾虑，可单独回退 wording 与 smoke，而不影响既有功能链。
风险与控制：若术语冻结不彻底，会继续出现 README、CLI note、doctor advice 各说各话；通过统一到 `generated root`、`project bridge config`、`bridge-only repair` 三个固定词汇，并让 smoke 直接覆盖完整修复故事，降低二次漂移。

### Batch V1.0-P7 - Lexical Cwd Path Stability（已完成）

目标：消除 CLI 在 symlink/project alias 场景下把 shell 里的词法路径无故折成物理路径的输出漂移，让 `init --json`、`doctor --json`、generated README connect snippets 等面向用户或 host 的路径，优先保留当前 shell 的 `PWD` 词法形式，而不是回退成 `/private/...` 这类底层真实路径。
依赖顺序：`T-161(Completed)`。
回滚策略：只新增“优先采用安全 `PWD`”的 cwd 解析 helper，并替换少数用户可见输出链路的 cwd 来源；若发现兼容性问题，可单独回退 helper 接入点，不影响 runtime 主执行链。
风险与控制：直接信任 `PWD` 会有伪造风险；通过仅在 `PWD` 为 absolute 且 `canonicalize(PWD) == canonicalize(current_dir)` 时才采用词法路径，其他情况一律回退 `current_dir()`，控制语义偏移。

### Batch V1.0-P6 - Init Report Bridge File Accounting（已完成）

目标：补齐 `init --json` 在 project bridge 路径上的文件变更可见性，让 `created_files/overwritten_files` 不再只覆盖 generated root 内文件，而是能稳定反映项目根 bridge config 与 `.gitignore` 的实际写入结果，方便自动化与 host 侧消费。
依赖顺序：`T-160(Completed)`。
回滚策略：仅扩展 `InitReport` 的文件清单填充逻辑，不改变 bridge config/gitignore 的实际写入语义；必要时可回退 report accounting，而不影响 `init` / `sync-project-config-only` 主链。
风险与控制：若把“preserved 现有文件”也误记成 overwritten，会误导自动化；通过只在实际写入发生时追加到 `created_files/overwritten_files`，保留 notes 继续表达 preserved/no-op，降低语义污染。

### Batch V1.0-P5 - Project Bridge Repair Path（已完成）

目标：把 `doctor.project_bridge.repair_command` 从“借 `--refresh-bootstrap` 顺带修”收口成真正的 bridge-only 修复路径，让已有 generated root 的项目桥接可以独立补齐或覆盖，而不去触碰 bootstrap 模板内容。
依赖顺序：`T-159(Completed)`。
回滚策略：仅新增显式 bridge-only repair 入口，并把 `doctor` repair command 改指向该入口；默认 `init`、`refresh-bootstrap`、`--sync-project-config` 的现有语义保持不变，必要时可单独回退新 flag 与 repair command。
风险与控制：若 bridge-only repair 对 root 合法性校验过松，可能把任意目录误当 generated root；通过要求 `--root-dir`、校验 manifest/legacy generated-root 形态、并保留 `agents/` 目录与 spec 加载校验，避免误指向普通目录。

### Batch V1.0-P4 - Project Bridge Diagnostics（已完成）

目标：把 `init --sync-project-config` 和 generated-root/refresh 路径补成真正可观察的 doctor 诊断面，让项目根能直接看见 bridge config 是否存在、当前指向哪个 root、是否位于项目内/项目外，以及该执行哪条精确 repair command，而不再靠猜 `config.toml` 或重复试错命令。
依赖顺序：`T-158(Completed)`。
回滚策略：仅扩展 `doctor` 的 report/rendering/smoke，不改变 `init`、runtime config merge 或现有 bootstrap drift 逻辑；必要时可单独回退 `project_bridge` 视图而不影响主执行链。
风险与控制：project bridge 诊断若把“当前 runtime 视图”和“项目 config 视图”混在一起，会制造误导；通过同时保留 configured/runtime 路径、显式 `status` 和 `repair_command`，并只在能识别 generated root 时生成修复建议，降低误报面。

### Batch V1.0-P3 - Custom Root Project Bridge Sync（已完成）

目标：解决 `init --root-dir <custom-root>` 后项目根命令面仍然不知道这个 root 的问题，补一个显式的 project bridge sync 入口，让 `validate/doctor/list-agents` 等命令能无额外 flags 指向 custom root，同时保持当前默认行为不变。
依赖顺序：`T-157(Completed)`。
回滚策略：仅新增显式 `init --sync-project-config` 路径，不改变默认 bootstrap init / refresh 的现有行为；必要时可回退该 flag 与对应 bridge-config 写入逻辑，不影响 runtime 主链。
风险与控制：自动改写项目 config 可能造成意外指向外部 root；通过只在显式 `--sync-project-config` 时启用、并在 notes 中写明 agents/state 指向，降低惊讶面。若 custom root 在项目内，gitignore 规则需避免过宽匹配；通过仅追加精确相对路径规则控制影响范围。

### Batch V1.0-P2 - Generated Root Manifest + Exact Repair Command（已完成）

目标：把 bootstrap drift 的识别与修复从“默认 `.mcp-subagent/bootstrap` 路径可用”推广到任意 `init` 生成的 root，并让 `doctor` 输出精确可执行的 repair command，而不是只给泛泛建议。这样 `init --root-dir <custom-root>` 场景下的 drift 也能被稳定识别、稳定修复。
依赖顺序：`T-156(Completed)`。
回滚策略：仅新增 generated-root manifest、diagnostic surface 和 repair command 输出；保留对旧默认 bootstrap 路径的兼容探测，不改 runtime 主执行链，也不要求用户立即迁移已有 root。
风险与控制：若 manifest 识别过宽，可能误把普通 `agents/` 目录当成 generated root；通过“manifest 优先 + 旧默认路径兼容 fallback”的双通道识别控制误报。repair command 统一输出显式 `--root-dir`，避免 cwd 依赖与文案歧义。

### Batch V1.0-P1 - Bootstrap Drift Repair Path（已完成）

目标：把现有“只能发现 bootstrap drift”的诊断能力补成“可安全修复 drift”的低摩擦入口，直接解决旧 bootstrap 仍携带 legacy `active_plan` 等漂移模板时，用户只能靠重初始化整套 root 才能恢复的问题。本批次只处理 bootstrap catalog 内建模板的刷新与修复提示，不扩展到 runtime、state layout 或新的存储契约。
依赖顺序：`T-155(Completed)`。
回滚策略：CLI 仅新增显式 `init --refresh-bootstrap` 修复入口，默认 `init` 和 `doctor` 行为不变；必要时可单独回退 refresh 分支和文档，不影响既有初始化/诊断主路径。
风险与控制：refresh 若误覆盖用户自定义 agent 会造成意外丢失；通过只重写当前 bootstrap root 中“文件名命中内建 catalog”的模板、保留自定义 agent、不自动创建新模板，降低破坏面。README/doctor 建议统一改成这条安全修复路径，避免继续把用户推回整套 `--force` 重初始化。

### Batch V1.0-P0 - Release Cutpoint + Stream/Status Regression（已完成）

目标：在 `V0.10-P1` 能力已完成的前提下，补齐发布切口上仍然缺失的两类保证：其一是 `status --json` 的稳定 contract 断言，其二是 `run/spawn/submit --stream` 至少有一条真实 smoke 链路覆盖，不让 CLI 表面能力只停留在单元测试层。与此同时清理 `PLAN.md` 中遗留的历史“当前优先”标记，保证计划面只保留一个真实当前批次。
依赖顺序：`T-154(Completed)`。
回滚策略：仅新增测试、smoke 和计划/文档收口，不变更 runtime 主执行链；若 smoke 断言过严，可回退脚本和测试而不影响功能面。
风险与控制：stream smoke 依赖 fake provider 输出路径，断言需聚焦稳定信号而不是具体时序；通过选择 `--json --stream` 的 wrapper 行和 `status --json` 的稳定字段，降低 flaky 风险。

### Batch V0.10-P0 - Spawn Accepted-only + Runtime Transparency（已完成）

目标：先消除“spawn 黑盒卡住”与“运行中不可观察”两类核心体验问题。首个切片先完成 `spawn` accepted-only 语义（同步路径不做 provider probe），随后补事件流/心跳/watch/stats，并补 `block_reason + logs --follow + waiting/watchdog events + stats phase splits + phase_progress view + phase filter/timeout + MCP phase watchdog + watch advice` 让阻塞可解释。
依赖顺序：`T-086(Completed) -> T-087(Completed) -> T-088(Completed) -> T-089(Completed) -> T-090(Completed) -> T-091(Completed) -> T-092(Completed) -> T-093(Completed) -> T-094(Completed) -> T-095(Completed) -> T-096(Completed) -> T-097(Completed) -> T-098(Completed) -> T-099(Completed) -> T-100(Completed) -> T-101(Completed) -> T-102(Completed) -> T-103(Completed) -> T-104(Completed) -> T-105(Completed) -> T-106(Completed) -> T-107(Completed) -> T-108(Completed) -> T-109(Completed) -> T-110(Completed) -> T-111(Completed)`。
回滚策略：`run_agent` 保持同步 probe 语义；`spawn_agent` 仅将 probe 后移到 worker，异常仍落盘同一 run 结构，必要时可回退到旧 `prepare_run + upfront probe` 路径。
风险与控制：provider 不可用从“同步拒绝”变成“异步失败”可能影响调用侧预期；通过保留明确 `error_message`（含 unavailable 原因）和测试覆盖（slow probe 快返、unavailable 异步失败）降低误解。

### Batch V0.10-P1 - Parser Bridge + Bootstrap Drift Guard + CLI Exposure（已完成）

目标：在不改 `RunPhase`、不改现有 run 目录布局、也不引入新持久化契约的前提下，优先收口三个真实痛点：`summary` 归一化桥接误判、bootstrap preset 漂移导致的上下文过量注入，以及已有流式/心跳/阻塞诊断能力尚未被 `run/spawn/status` 直接暴露。当前仓库术语保持统一使用 `RunPhase`、`parse_status`、`delegation_context`、`context_mode`、`memory_sources`、`StableScratch`。
依赖顺序：`T-150(Completed) -> T-151(Completed) -> T-152(Completed) -> T-153(Completed)`。
回滚策略：parser 收口仅增强 `best_effort` 主路径并保留 strict 旧语义；bootstrap/preset 治理不改现有文件名与 agent spec 结构；CLI `--stream` 仅做增量 flag 暴露，不改变默认非流式行为。
风险与控制：放宽裸 JSON 识别可能掩盖真正的格式违规；通过继续保留 `parse_status`、native artifacts、以及占位 sentinel 污染回归测试控制误判。旧 workspace 中已生成的 bootstrap 文件不会被 `init` 自动重写；通过新增 drift 检查/提示而不是静默覆盖降低意外变更风险。CLI 流式输出可能影响脚本消费；通过 `--stream` 显式 opt-in、保持现有默认输出不变来控制兼容性。

## Execution Strategy (v0.9 Current)

### Batch V0.9-P0 - Delegation Minimal + Native-first

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

### Batch V0.8-P0 - First Success Path

目标：完成 `connect-snippet + init README + smoke_v08/CI + release docs/changelog/version + real examples/onboarding + CI reliability fixes + summary parsing robustness` 收口，让用户首次接入、发布切点和示例落地都可复制可验证。
依赖顺序：`T-059 -> T-060 -> T-061 -> T-062 -> T-064 -> T-065 -> T-066 -> T-067 -> T-068 -> T-069 -> T-070 -> T-071`。
回滚策略：新增命令面与模板升级均保持向后兼容，不影响既有 `mcp/doctor/validate/run/spawn` 主链。
风险与控制：路径绝对化与 shell 转义实现不当会导致复制失败；smoke 误依赖本机真实 codex 会导致 CI 不稳定。通过单测覆盖 host 模板、绝对路径和含空格路径转义，并在 smoke 中使用 fake codex runner 固定回归路径。

## Execution Strategy (Module Batches)

### Batch A - Runtime 可直接操作

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

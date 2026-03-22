# 迁移进度

English version: [MIGRATION_PROGRESS.md](./MIGRATION_PROGRESS.md)。

本文件汇总 `gitnexus-rs` 的迁移进度详情（中文版本），从 README 中拆分出来，便于独立维护。

Last updated: `2026-03-14`

## 中文快照（2026-03-14）

基于 Rust（`/src`）与 TypeScript（`/GitNexus/gitnexus/src`）实现的并行对比：

- 整体迁移进度（相对 TypeScript 的功能对齐）：**约 45% 到 55%**，目前更接近 **约 50%**，但还不能说已经稳定跨过“半数完成”。
- 阶段状态：
  - **阶段 1**：已完成。
  - **阶段 2**：推进较深，但尚未达到 TS 等价。
  - **阶段 3**：已具备 Rust `analyze` 的 Kuzu / FTS 基线，但与 TS 的完整等价仍未完成。
  - **阶段 4**：已进入词法检索基线阶段，但 semantic / hybrid parity 仍明显缺失。
  - **阶段 5**：已有原生基线，但兼容性与行为对齐仍未完成。
  - **阶段 6**：尚未完成。

## 审计范围与证据口径

- 本审计跟踪的是 `src` 下 Rust CLI 主体和影响 parity 的运行时能力，不是对 TS 所有辅助 CLI 入口做逐项迁移认定。
- TS 基线：`GitNexus/gitnexus/src`
- Rust 对照面：`src`
- 证据来源：GitNexus MCP（`query/context/cypher`）+ `src` 与 `GitNexus/gitnexus/src` 文件系统直接检查。
- `gitnexus-rs` MCP 索引对本次审计是最新的。
- 上游 `GitNexus` MCP 索引落后 HEAD `2` 个 commit，因此 TS 基线判断采用“`MCP + 文件系统检查`”口径，而不是只依赖 MCP。
- TS 侧 `ai-context`、`lazy-action`、`tool` 仍属于当前迁移跟踪范围之外的辅助入口；它们没有对应的 `src/commands` Rust 入口。

### 命令迁移矩阵

- Rust 原生默认路径：
  - `analyze`、`list`、`status`、`clean`
  - `setup`、`wiki`、`augment`、`eval-server`
- Rust-first（入口仍经过 delegate wrapper）：
  - `query`、`context`、`impact`、`cypher`、`detect-changes`、`rename`、`mcp`、`serve`
- 当前迁移范围外的 TS 辅助入口：
  - `ai-context`、`lazy-action`、`tool`

备注：

- `query/context/impact/cypher/detect-changes/rename/mcp/serve` 仍先经过 `run_ts_cli(...)` 入口，但会优先被 Rust 原生分发（`local_tools::try_run_native_tool`）截获。
- `augment` 与 `eval-server` 已默认走 Rust 原生路径，TS 回退仅保留为显式环境变量开关：
  - `GITNEXUS_USE_TS_AUGMENT=1`
  - `GITNEXUS_USE_TS_EVAL_SERVER=1`
- 查询类工具当前是原生 Kuzu bridge 优先、启发式回退兜底，还不是 TS 等价能力。
- Rust `/api/search` 现在也会优先尝试原生 Kuzu FTS，再回退到本地 `query` 基线路径。

### Rust 已有能力（已确认）

- Git + 存储基础能力：
  - Git 仓库检测/根目录解析/commit 查询
  - `.gitnexus/meta.json` 与 `~/.gitnexus/registry.json` 兼容
- 摄取基线能力：
  - 摄取模块已拆分为 `scan`、`symbols`、`relations`、`summaries`、`graph`、`parser_loader`
  - 按忽略规则与文件大小规则扫描仓库
  - 启发式符号提取
  - 启发式 `DEFINES`、`IMPORTS`、`CALLS`、`EXTENDS`、`IMPLEMENTS` 边
  - 分阶段分块并行解析（worker-pool）+ 进度回调 + 有界结果背压 + 首错停止继续调度
  - TypeScript / JavaScript 路由文件的 route-derived `CALLS` 基线提取
  - TypeScript / JavaScript、Go、PHP、C# 的 import 解析基线
  - 启发式 community/process 摘要
  - 摄取报告输出（`.gitnexus/ingestion.json`）

### 相比 TypeScript 的主要差距

- Rust 运行时已覆盖核心工具命令，但仍是启发式实现，尚未达到 TS/Kuzu 的能力深度。
- 当前最大实际差距仍在运行时深度：
  - Rust 的工具查询路径现在已包含原生 Kuzu bridge（`query` / `context` / `impact` / `cypher`）并带有 `.gitnexus/graph.json` 回退；Rust `analyze` 现已补上 Kuzu/FTS 物化基线，但 schema/loader/增量更新 仍未达到 TS 深度。
  - TS `analyze` 会构建 Kuzu + FTS + 可选 embeddings；Rust `analyze` 现在会执行 ingestion、Kuzu 物化、FTS 构建并写入 graph cache / meta / report，但 `--embeddings` 仍未实现。
  - TS 的 wiki 工作流包含 LLM 配置解析、交互式 setup、更丰富的生成流程与可选 gist 发布；Rust `wiki` 目前仍是静态基线实现。
  - Rust `/api/search` 当前会先尝试原生 Kuzu FTS，再回退本地 `query` 基线，但仍不是 TS 的 hybrid search 行为。
  - Rust MCP / HTTP serving 已有原生实现，但 resources / prompts / setup / staleness guidance 仍较 TS 简化。

### 仍未完成的对齐项（代码级审计）

- Rust 运行时深度未对齐：
  - `analyze` 阶段的增量重载 / 更新语义与完整性校验仍未对齐
  - Kuzu schema / loader / 元数据覆盖仍未与 TS 完全对齐
  - embeddings 生成与语义检索对齐
  - hybrid search 排序 / 融合策略对齐
  - wiki 完整生成流水线对齐（LLM 模块生成 / 增量重建 / gist 发布）
- 已有 Rust 原生基线，但兼容性或行为对齐仍未完成：
  - `augment`：宿主工具矩阵下的 hook 输出兼容性仍未完全硬化
  - `eval-server`：多客户端 / eval client 兼容性仍未完全硬化
  - `wiki`：Rust 基线当前可生成静态 markdown 与架构摘要，但尚不包含 TS 侧 LLM/增量/gist 能力。
  - `query`：Rust 结果排序与 TS 的 BM25+semantic 混合检索深度仍有差距。
  - `cypher`：Rust 现已包含原生 Kuzu bridge；其回退 evaluator 仍是安全只读子集，且尚未保证与 TS 完全等价。
  - `/api/search`：Rust 服务当前先尝试原生 Kuzu FTS，再回退本地 `query` 基线路径，不是 TS 的 hybrid search 行为。
  - MCP setup/resource 对齐：Rust 已有核心 resources/tools/prompts，但 `gitnexus://setup` 与 staleness 提示仍较 TS 简化。

### 中文执行摘要

- 相对 TypeScript 的迁移对齐度当前约 **45% 到 55%**。
- Rust 已有稳定的阶段 1 基线，并且 `augment` / `eval-server` 也已经默认走 Rust 原生路径。
- Rust 现已包含 `setup/query/context/impact/cypher/detect-changes/rename/mcp/serve` 的 Rust-first 基线实现。
- 阶段 2 已有真实进展，包括 ingestion 模块拆分、分块进度解析、有界背压、TS/JS route 提取基线，以及 TS/JS、Go、PHP、C# import 解析基线。
- 阶段 3 已推进到 `analyze` 期 Kuzu 物化 + FTS 构建 + graph cache 输出的原生基线。
- Rust 端当前最大缺口转为 `embeddings -> hybrid search -> richer wiki/MCP parity`，以及 `analyze` 的增量更新与完整等价行为。
- 实际含义：命令表面已大体原生化，但高级图智能仍未达到 TS 侧能力深度，主要缺口已转到 embeddings、hybrid search、增量更新与更完整的运行时对齐。

# 迁移进度

English version: [MIGRATION_PROGRESS.md](./MIGRATION_PROGRESS.md)。

本文件汇总 `gitnexus-rs` 的迁移进度详情（中文版本），从 README 中拆分出来，便于独立维护。

Last updated: `2026-03-12`

## 中文快照（2026-03-12）

基于 Rust（`/src`）与 TypeScript（`/GitNexus/gitnexus/src`）实现的并行对比：

- 整体迁移进度（相对 TypeScript 的功能对齐）：**约 35% 到 45%**
- 阶段状态：
  - **阶段 1**：基本完成
  - **阶段 2**：部分完成（摄取基线可用，但尚未对齐）
  - **阶段 3**：已启动引导（本地图运行时 + 原生工具命令基线）
  - **阶段 4-6**：尚未迁移

### 命令迁移矩阵

- Rust 原生：
  - `analyze`、`list`、`status`、`clean`
  - `setup`、`query`、`context`、`impact`、`cypher`、`detect-changes`、`rename`、`mcp`、`serve`、`wiki`（启发式/本地图运行时基线）
- 转发到 TypeScript：
  - `augment`、`eval-server`

### Rust 已有能力（已确认）

- Git + 存储基础能力：
  - Git 仓库检测/根目录解析/commit 查询
  - `.gitnexus/meta.json` 与 `~/.gitnexus/registry.json` 兼容
- 摄取基线能力：
  - 按忽略规则与文件大小规则扫描仓库
  - 启发式符号提取
  - 启发式 `DEFINES`、`IMPORTS`、`CALLS`、`EXTENDS`、`IMPLEMENTS` 边
  - 启发式 community/process 摘要
  - 摄取报告输出（`.gitnexus/ingestion.json`）

### 相比 TypeScript 的主要差距

- Rust 运行时已覆盖核心工具命令，但仍是启发式实现，尚未达到 TS/Kuzu 的能力深度。
- 当前最大实际差距仍在运行时深度：
  - Rust 的工具查询路径现在已包含原生 Kuzu bridge（`query` / `context` / `impact` / `cypher`）并带有 `.gitnexus/graph.json` 回退，但 Rust `analyze` 仍未提供 TS 级别的 Kuzu/FTS/embeddings 物化能力。
  - TS `analyze` 会构建 Kuzu + FTS + 可选 embeddings；Rust `analyze` 当前主要写入元数据/摄取报告，并提示 embeddings 预览版未实现。
  - TS 的 wiki/augmentation/eval 工作流已较完整；Rust 仍委托 `augment`/`eval-server`，`wiki` 目前是基线实现。

### 仍未迁移能力（代码级审计）

- 明确未迁移（仍由 TS 委托）：
  - `augment`
  - `eval-server`
- Rust 运行时深度未迁移：
  - Kuzu 图持久化与查询运行时对齐
  - FTS 索引构建/查询对齐
  - embeddings 生成与语义检索对齐
  - wiki 完整生成流水线对齐（LLM 模块生成/增量重建/gist 发布）
- 部分迁移（已有 Rust 基线，但仍有对齐缺口）：
  - `wiki`：Rust 基线当前可生成静态 markdown 与架构摘要，但尚不包含 TS 侧 LLM/增量/gist 能力。
  - `query`：Rust 结果排序与 TS 的 BM25+semantic 混合检索深度仍有差距。
  - `cypher`：Rust 现已包含原生 Kuzu bridge；其回退 evaluator 仍是安全只读子集，且尚未保证与 TS 完全等价。
  - `/api/search`：Rust 服务当前经本地 `query` 基线路径，不是 TS 的 hybrid search 行为。
  - MCP setup/resource 对齐：Rust 已有核心 resources/tools/prompts，但 `gitnexus://setup` 与 staleness 提示仍较 TS 简化。

### 中文执行摘要

- 相对 TypeScript 的迁移对齐度当前约 **35% 到 45%**。
- Rust 已有稳定的阶段 1 基线（`analyze/list/status/clean`，以及 git/存储/注册表兼容）。
- Rust 现已包含 `setup/query/context/impact/cypher/detect-changes/rename/mcp/serve` 的原生基线实现。
- 阶段 2/3 的能力已可用，但仍属启发式与部分实现。
- Rust 端当前最大缺口仍是 Kuzu 图运行时对齐、FTS/embeddings 对齐，以及 wiki/augment/eval 工作流对齐。
- 实际含义：命令表面已大体原生化，但高级图智能仍未达到 TS 侧能力深度（尤其 Kuzu 物化、FTS、embeddings）。

# gitnexus-rs

英文版请见 [README.md](./README.md)。

`gitnexus` 包的 Rust 迁移基线。

该 crate 有意与现有 TypeScript 包（`/gitnexus`）并存，以便在不破坏当前工作流的前提下逐步迁移。

本仓库包含 AI 驱动的迁移工作，目标是在性能受限场景（例如本地低内存或有限算力环境）下，优先通过轻量级 Rust 原生路径保持项目可运行。

## 许可

- 本仓库采用 `PolyForm Noncommercial License 1.0.0`，详见 [LICENSE](./LICENSE)。
- 仓库包含来自 GitNexus 的迁移/衍生工作；必需归属声明见 [NOTICE](./NOTICE)。
- 商业用途需要相关权利持有者的单独授权。

## Rust 已实现功能（阶段 1）

- Rust 原生 CLI 命令：`analyze`、`list`、`status`、`clean`、`query`、`context`、`impact`、`cypher`、`detect-changes`、`rename`、`mcp`、`serve`
- 为兼容性转发到 TypeScript CLI 的命令：
  - `setup`、`wiki`
  - `augment`、`eval-server`
- Git 辅助能力：
  - 检测 git 仓库
  - 解析 git 根目录
  - 读取当前 commit
- 与当前 JSON 布局兼容的存储与注册表：
  - `.gitnexus/meta.json`
  - `~/.gitnexus/registry.json`

## Rust 已实现功能（阶段 2 - 部分）

- 摄取（ingestion）流水线基础：
  - 基于忽略规则扫描仓库
  - 大文件过滤（`>512KB`）
  - 支持源码扩展名的语言识别
  - 结构图提取（`Folder`/`File` + `CONTAINS`）
  - 基于启发式的符号提取基线（`Function`/`Class`/`Interface`/`Struct`/`Enum`/`Trait`）
  - 基于启发式的 TypeScript/JavaScript 相对导入提取（`IMPORTS`）
  - 基于启发式的 TypeScript/JavaScript 调用提取（`CALLS`）
  - 基于启发式的 TypeScript/JavaScript 继承提取（`EXTENDS`/`IMPLEMENTS`）
  - 从 `File` 节点到提取符号的 `DEFINES` 边
- 摄取报告输出：
  - `.gitnexus/ingestion.json`

## Rust 已实现功能（阶段 3 - 引导版，部分）

- 面向原生工具命令的本地图运行时引导：
  - 原生 `query`/`context`/`impact`/`cypher`/`detect_changes`/`rename` 不再依赖 TS 转发
  - 按需构建的图缓存持久化到 `.gitnexus/graph.json`
  - 回退行为：若缓存不存在，则从 Rust 摄取流水线构建并本地缓存
- MCP 引导实现：
  - 原生 `mcp` stdio 服务基于 Rust MCP SDK（`rmcp`）和 `transport::stdio` 实现
  - tool/resource/prompt 处理通过 Rust `ServerHandler` 实现（initialize/tools/resources/prompts/ping）
  - 工具执行优先路由到可用的 Rust 原生命令实现
- HTTP serve 引导实现：
  - 原生 `serve` 命令已用 Rust 实现（`axum` + tokio listener，默认 `127.0.0.1:4747`）
  - REST 基线端点覆盖 repos/graph/query/search/file/processes/clusters
  - `/api/mcp` 通过 `rmcp` Streamable HTTP 传输挂载（`StreamableHttpService`，有状态会话）
  - 会话生命周期与 SSE priming 行为交由 rmcp 传输层处理
  - 已移除 `local_serve.rs` 中旧的手写 TCP/MCP 路径，避免双轨传输实现
- 原生 `cypher` 当前支持安全的只读子集：
  - `MATCH ... RETURN ... [LIMIT n]`
  - 节点模式与单跳关系模式
  - `WHERE` 的 `=` / `CONTAINS`（可通过 `AND` 组合）
  - `COUNT(*)` 与列投影（`AS`）
  - 显式阻止写操作

## 当前行为

- `analyze` 当前执行结构 + 解析引导流程：
  - 查找仓库根目录
  - 按忽略规则与体积规则扫描仓库文件
  - 构建结构图与文件到符号的 `DEFINES` 链接
  - 添加启发式 TypeScript/JavaScript 相对导入 `IMPORTS` 边
  - 添加启发式 TypeScript/JavaScript 函数/类调用 `CALLS` 边
  - 添加启发式 TypeScript/JavaScript 继承 `EXTENDS`/`IMPLEMENTS` 边
  - 写入/更新 `.gitnexus/meta.json` 的文件/节点/边统计（`communities` / `processes` 当前为基于关系提取的启发式估算）
  - 写入 `.gitnexus/ingestion.json` 摄取统计
  - 将仓库注册到 `~/.gitnexus/registry.json`
  - 确保 `.gitnexus` 被写入 `.gitignore`
- `query` / `context` / `impact` / `cypher` / `detect-changes` / `rename` 现已通过 Rust 原生本地图运行时执行（缓存为 `.gitnexus/graph.json`）。
- `mcp` 现已通过 rmcp 支持的 Rust 原生 stdio 传输执行。
- `serve` 现已通过 Rust 原生 HTTP 服务基线执行。
- `/api/mcp` 当前运行在 rmcp Streamable HTTP 传输之上（有状态会话、SSE priming、`mcp-session-id` 生命周期由 SDK 传输层处理）。
- `local_serve.rs` 不再保留此前自定义 `/api/mcp` session/SSE/TCP 回退实现；MCP HTTP 仅保留 rmcp。
- 未迁移命令仍在可用时转发到 TypeScript 实现（优先本地源码/构建，其次回退 `npx gitnexus@latest`）。
- 图摄取深度对齐、Kuzu 加载、FTS、embeddings 和 wiki 生成尚未迁移。

## 迁移状态快照（2026-03-11）

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
  - `query`、`context`、`impact`、`cypher`、`detect-changes`、`rename`、`mcp`、`serve`（启发式/本地图运行时基线）
- 转发到 TypeScript：
  - `setup`、`wiki`
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
- TypeScript 实现目前已提供：
  - 完整的 Tree-sitter 摄取流水线
  - Kuzu 图物化 + schema 加载
  - 全文检索（FTS）+ embeddings
  - 完整的 Express + StreamableHTTP MCP 传输行为
  - wiki 生成
  - 由本地图查询支撑的高级图工具（例如 `detect_changes`、`rename`）

### 中文执行摘要

- 相对 TypeScript 的迁移对齐度当前约 **35% 到 45%**。
- Rust 已有稳定的阶段 1 基线（`analyze/list/status/clean`，以及 git/存储/注册表兼容）。
- Rust 现已包含 `query/context/impact/cypher/detect-changes/rename/mcp/serve` 的原生基线实现。
- 阶段 2/3 的能力已可用，但仍属启发式与部分实现。
- Rust 端当前最大缺口是 Kuzu 图持久化、FTS、embeddings 与 wiki 生成。
- 实际含义：命令表面已经更原生，但高级图智能仍主要依赖 TS/Kuzu 能力。

## 为什么是这种形态

TypeScript 版本包含大型多阶段流水线（Tree-sitter 解析、图关系提取、Kuzu 物化、向量生成、MCP/HTTP 服务）。完整等价的 Rust 重写应分阶段安全推进，以维持行为对齐并避免回归。

## 建议迁移阶段顺序

1. 迁移存储 + git + CLI 基础能力（已完成）
2. 迁移摄取流水线（结构/解析/import/call/heritage/community/process）
3. 迁移图持久化（Kuzu 适配器 + schema + loaders）
4. 迁移检索与 embeddings
5. 迁移 MCP 服务与 HTTP 服务
6. 基于现有 TS fixtures 与集成测试做对齐验证

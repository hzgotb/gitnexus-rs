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

- Rust 原生 CLI 命令：`analyze`、`list`、`status`、`clean`、`setup`、`query`、`context`、`impact`、`cypher`、`detect-changes`、`rename`、`mcp`、`serve`、`wiki`、`augment`、`eval-server`
- 阶段 1 命令可选 TypeScript 回退（默认关闭，显式开启）：
  - `GITNEXUS_USE_TS_AUGMENT=1`
  - `GITNEXUS_USE_TS_EVAL_SERVER=1`
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
  - 导入提取的 advanced resolution 基线（`IMPORTS`）：
    - 相对导入（`./`、`../`）
    - `tsconfig` `paths` 别名改写
    - `baseUrl` 绝对路径 specifier 解析
    - 基于 suffix-index 的 module-context path-like 回退解析
    - `go.mod` module-path 本地包解析（Go）
    - `composer.json` `autoload.psr-4` 命名空间解析（PHP）
    - `.csproj` `RootNamespace` 命名空间解析（C#）
    - 关系能力拆分基线：Go/C# 仅做 `IMPORTS`；TS/JS 与 PHP 已具备路由派生 `CALLS` 提取基线
  - 基于启发式的 TypeScript/JavaScript 调用提取（`CALLS`）
  - 基于启发式的 TypeScript/JavaScript 路由调用提取（`CALLS`，面向 route-like 文件）：
    - 路由方法模式：`.get/.post/.put/.patch/.delete/.options/.head/.all/.use`
    - handler 形态：标识符、成员表达式尾部、数组 handlers、wrapper 调用参数
    - 产出文件节点到 handler 符号的 `CALLS` 边（`File:<route file> -> handler`）
  - 基于启发式的 Laravel 路由调用提取（`CALLS`，PHP 路由文件）：
    - `Route::...([Controller::class, 'method'])`
    - `Route::resource(...)` / `Route::apiResource(...)` 映射到控制器约定动作
  - 基于启发式的 TypeScript/JavaScript 继承提取（`EXTENDS`/`IMPLEMENTS`）
  - 从 `File` 节点到提取符号的 `DEFINES` 边
  - 有界分块执行基线：
    - 通过 worker-pool 按阶段并行处理 `symbols` / `imports` / `calls` / `heritage` 分块
    - 内存/文件预算控制（`PARSE_CHUNK_MAX_BYTES`、`PARSE_CHUNK_MAX_FILES`）
    - 有界 worker 结果背压队列（`sync_channel`），限制 chunk 结果排队内存
    - 首个 chunk 解析错误后停止继续调度新 chunk（fail-fast）
    - 进度回调 API（`run_ingestion_pipeline_with_progress`）
    - 分块 `import` 解析保持全仓上下文，避免跨 chunk 漏解析
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
  - 会话生命周期交由 rmcp 传输层处理，Rust 侧增加了传输兼容性归一化
  - 已移除 `local_serve.rs` 中旧的手写 TCP/MCP 路径，避免双轨传输实现
- 原生 `cypher` 当前支持安全的只读子集：
  - `MATCH ... RETURN ... [LIMIT n]`
  - 节点模式与单跳关系模式
  - `WHERE` 的 `=` / `CONTAINS`（可通过 `AND` 组合）
  - `COUNT(*)` 与列投影（`AS`）
  - 显式阻止写操作
- `query` / `context` / `impact` / `cypher` 的 Kuzu bridge 基线：
  - 主路径：Rust 原生 Kuzu bridge，读取本地 `.gitnexus/kuzu`
  - 输出兼容：`query` / `context` / `impact` 保持既有 JSON schema；`cypher` 保持 `{markdown,row_count}`
  - 回退路径：若本地 Kuzu DB 不可用，Rust 自动回退到本地 `.gitnexus/graph.json` 启发式运行时（`cypher` 回退到只读子集 evaluator）

## 当前行为

- `analyze` 当前执行结构 + 解析引导流程：
  - 查找仓库根目录
  - 按忽略规则与体积规则扫描仓库文件
  - 构建结构图与文件到符号的 `DEFINES` 链接
  - 添加 `IMPORTS` 边（advanced import resolution 基线）：
    - TypeScript/JavaScript：相对导入 + `tsconfig` `paths` + `baseUrl` + module-context suffix 回退
    - Go：`go.mod` module-path 本地包解析
    - PHP：`composer.json` `autoload.psr-4` 命名空间解析
    - C#：`.csproj` `RootNamespace` 命名空间解析
  - 添加启发式 TypeScript/JavaScript 函数/类调用 `CALLS` 边
  - 为 TypeScript/JavaScript route-like 文件添加路由派生 `CALLS` 边（路由文件 -> 已解析 handler）
  - 为 Laravel 路由文件（`routes/*.php`）添加路由派生 `CALLS` 边（路由文件 -> 控制器方法）
  - 添加启发式 TypeScript/JavaScript 继承 `EXTENDS`/`IMPLEMENTS` 边
  - 以有界 worker-pool 分块方式执行解析（按阶段），并支持可选进度回调、结果背压与首错停止调度
  - 写入/更新 `.gitnexus/meta.json` 的文件/节点/边统计（`communities` / `processes` 当前为基于关系提取的启发式估算）
  - 写入 `.gitnexus/ingestion.json` 摄取统计
  - 将仓库注册到 `~/.gitnexus/registry.json`
  - 确保 `.gitnexus` 被写入 `.gitignore`
- `query` / `context` / `impact` / `cypher` / `detect-changes` / `rename` 现已通过 Rust 原生命令路径执行。
- `query` / `context` / `impact` / `cypher` 在存在 `.gitnexus/kuzu` 时优先走 Rust 原生 Kuzu bridge；若 Kuzu 不可用则自动回退到本地 `.gitnexus/graph.json` 启发式路径。
- `setup` 现已具备 Rust 原生基线（全局编辑器 MCP 配置合并、skills 安装、Claude hooks 合并）。
- 原生 `impact` 现已支持关系过滤能力基线：
  - `relationTypes` / `relation_types`：用于过滤遍历关系类型（`CALLS`、`IMPORTS`、`EXTENDS`、`IMPLEMENTS`）
  - `minConfidence` / `min_confidence`：用于按关系置信度阈值过滤（`0..1`）
  - 可通过 CLI 参数（`--relation-types`、`--min-confidence`）和 MCP 工具参数使用
- `mcp` 现已通过 rmcp 支持的 Rust 原生 stdio 传输执行。
- `serve` 现已通过 Rust 原生 HTTP 服务基线执行。
- `/api/mcp` 当前运行在 rmcp Streamable HTTP 传输之上（有状态会话、`mcp-session-id` 生命周期由 SDK 传输层处理）。
- `/api/mcp` 传输兼容性加固（2026-03-12）：
  - 对 MCP 客户端请求头做归一化（`Accept`/`Content-Type`），降低严格协商失败概率
  - 关闭 SSE 空 priming 事件（`sse_retry: None`），提升严格解码客户端兼容性
  - 为 MCP 响应兜底补齐 `Content-Type`，避免 `Unexpected content type: None` 错误
- `local_serve.rs` 不再保留此前自定义 `/api/mcp` session/SSE/TCP 回退实现；MCP HTTP 仅保留 rmcp。
- `augment` 现已具备 Rust 原生快路径（短输入直接返回、失败静默、兼容 hook 的 stderr 输出），可通过 `GITNEXUS_USE_TS_AUGMENT=1` 显式回退 TS。
- `eval-server` 现已具备 Rust 原生 HTTP 实现（`/tool/:name`、`/health`、`/shutdown`、idle-timeout 自动退出），可通过 `GITNEXUS_USE_TS_EVAL_SERVER=1` 显式回退 TS。
- Phase 1 对齐基线现已补充单元测试覆盖：`augment` 输出整形、`eval-server` formatter/next-step hint。
- Phase 1 对齐基线现已补充集成级契约测试覆盖：
  - `eval-server` 真实 HTTP 端点行为（`/health`、`/tool/:name`、`/shutdown`）
  - `augment` hook 行为（`stderr` 输出、短输入 no-op、后端失败静默）
- Phase 2 基线已开始进行 ingestion 模块拆分（保持行为不变）：
  - `src/ingestion/scan.rs`（文件系统扫描 + 语言识别/忽略规则）
  - `src/ingestion/symbols.rs`（多语言符号抽取）
  - `src/ingestion/relations.rs`（import/call/heritage 解析链路 + 关系解析 helper + advanced TS/JS import resolution 基线）
  - `src/ingestion/summaries.rs`（community/process 摘要构建）
  - `src/ingestion/graph.rs`（结构图节点/边物化）
  - `src/ingestion/parser_loader.rs`（语言能力矩阵 + parser strategy 抽象基线）
  - `run_ingestion_pipeline_with_progress` 已提供分块执行基线（有界 chunk planner + worker-pool 分阶段执行 + 分阶段进度回调 + 有界结果背压 + 首错停止调度）
- Rust 现已包含原生 `wiki` 基线，可在 `.gitnexus/wiki` 生成静态 markdown 文档。
- `analyze` 里的 Rust 原生 Kuzu 物化/加载、FTS、embeddings 与 wiki 的完整 LLM 对齐能力仍未迁移。

## 迁移进度

完整迁移进度（含命令矩阵、能力差距）已独立到：

- [MIGRATION_PROGRESS.zh-CN.md](./MIGRATION_PROGRESS.zh-CN.md)
- [MIGRATION_PROGRESS.md](./MIGRATION_PROGRESS.md)

## 容器化测试验证（不污染主机环境）

如果你只需要做验证，不希望在主机安装 `cmake` 等原生构建依赖，可以在容器里运行测试：

1. 首次构建测试镜像：
   `docker build -f Dockerfile.test -t gitnexus-rs-test .`
2. 在容器中执行默认验证（`cargo test`）：
   `./scripts/test-in-container.sh`
3. 执行指定 cargo 子命令：
   `./scripts/test-in-container.sh test -- --nocapture`

说明：

- 脚本会在本地不存在时自动构建 `gitnexus-rs-test` 镜像。
- 默认使用 `CARGO_TARGET_DIR=/tmp/gitnexus-target`，避免污染主机 `target/`。
- 可通过环境变量覆盖：
  - `CONTAINER_ENGINE`（`docker`/`podman`）
  - `GITNEXUS_TEST_IMAGE`
  - `GITNEXUS_TEST_PLATFORM`（例如 `linux/arm64`）
  - `GITNEXUS_TEST_REBUILD=1`（强制重建镜像）

## 为什么是这种形态

TypeScript 版本包含大型多阶段流水线（Tree-sitter 解析、图关系提取、Kuzu 物化、向量生成、MCP/HTTP 服务）。完整等价的 Rust 重写应分阶段安全推进，以维持行为对齐并避免回归。

## 建议迁移阶段顺序

1. 迁移存储 + git + CLI 基础能力（已完成）
2. 迁移摄取流水线（结构/解析/import/call/heritage/community/process）
3. 迁移图持久化（Kuzu 适配器 + schema + loaders）
4. 迁移检索与 embeddings
5. 迁移 MCP 服务与 HTTP 服务
6. 基于现有 TS fixtures 与集成测试做对齐验证

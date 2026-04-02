# GitNexus Runner Repo Overview

## 1. 仓库定位

这个仓库不是 GitNexus 本体，而是一个“容器运行与分析编排层”。  
核心目标是用 Zig CLI 统一管理以下流程：

- 维护需要挂载到容器的 repo 清单
- 启动/重建 GitNexus 容器
- 在容器内对 `/repos/*` 执行 `gitnexus analyze`

---

## 2. 当前目录结构（核心部分）

```text
.
├── packages/cli/src/
│   ├── add_repo.zig
│   ├── start.zig
│   └── analyze.zig
├── Dockerfile
├── repos.json
├── registry.json
├── build_release_all.sh
├── build.zig
└── mise.toml
```

---

## 3. 三个 CLI 的职责

### `add_repo`

用途：维护 `repos.json` 中的 repo 映射关系。  
输入：`<source-dir> [dest-name]`

主要逻辑：

- 将 source 路径转换为绝对路径并归一化
- 默认 `dest-name` 为目录 basename
- 检查重复/冲突：
  - 同一 source + 同一 dest：跳过重复追加
  - 同一 source + 不同 dest：报冲突
  - 同一 dest + 不同 source：报冲突
- 原子写回 `repos.json`

`repos.json` 的 repo 条目格式为：

```json
["/abs/host/path", "container_dir_name"]
```

---

### `start`

用途：启动 GitNexus 容器并挂载 registry + repos。  
关键参数：

- `--tag/-t`：指定镜像 tag（`gitnexus:<tag>`）
- `--port/-p`：指定宿主机绑定端口
- `--registry/-r`：指定 `registry.json` 路径（绝对或相对）

主要逻辑：

- 读取并校验 `registry.json`
- 读取并校验 `repos.json`（要求 `repos` 数组，每项 `[host, dest]`）
- 将 repos 挂载为 `-v <host_abs>:/repos/<dest>`
- 检查旧容器（`gitnexus` 或 `gitnexus-*`）并给出交互模式：
  - `n` 新建容器
  - `r` 重建已有容器（支持上下键选择目标）
  - `c` 取消
- 新建模式下容器名带随机后缀（支持多实例并行）
- 通过 `docker run` 启动，必要时配置端口映射与 `SERVE_PORT`

---

### `analyze`

用途：在宿主机触发容器内 `gitnexus analyze`。  
关键参数：

- `--container/-c`：指定容器名或 ID
- `--repo/-r`：指定单个 repo
- `--all/-a`：对容器内 `/repos/*` 全量执行
- `--jobs/-j`：`--all` 场景并发数
- `--` 之后参数透传给 `gitnexus analyze`

主要逻辑：

- 先选容器，再选 repo（多 repo 场景支持上下键选择）
- 单仓模式：一次 `docker exec`
- 全仓模式：并发启动多个 analyze 任务
- TTY 场景显示面板化进度（状态、并发、失败数）
- 解析输出中的 summary（例如 nodes/edges）并显示在 repo 后

---

## 4. 配置与数据文件

### `repos.json`

- 作用：声明宿主机 repo 到容器内 `/repos/<name>` 的挂载清单
- 当前结构：`{ "repos": [ [host_path, dest_name], ... ] }`

### `registry.json`

- 作用：GitNexus 索引结果与统计信息
- 典型字段：`name`, `path`, `indexedAt`, `stats.nodes`, `stats.edges` 等

---

## 5. 构建与发布

### `build_release_all.sh`

- 使用 `zig` 或 `mise x -- zig`
- 直接编译 `packages/cli/src/*.zig`
- 交叉构建多个平台目标，产物输出到 `dist/`

### `mise.toml`

- 配置 Zig 工具链
- 将 `./zig-out/bin` 注入 PATH，便于在当前仓库终端直接调用 CLI

---

## 6. 典型工作流

1. `add_repo <src> [dest]` 维护挂载清单  
2. `start [--tag ...] [--port ...] [--registry ...]` 启动或重建容器  
3. `analyze [--container ...] [--repo ... | --all -j N] [-- ...]` 执行分析  

---

## 7. 当前已知状态（2026-03-26）

- 当前源码目录已是 `packages/cli/src/*`
- `build_release_all.sh` 已指向该目录，可用于发布构建
- `build.zig` 仍引用 `apps/.../main.zig`，直接 `zig build` 会因路径不存在而失败

如果要让 `zig build` 恢复可用，需要把 `build.zig` 的入口路径同步到 `packages/cli/src/*`。

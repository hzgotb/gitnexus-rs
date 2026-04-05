# Migration Progress (TS Baseline)

中文版请见 [MIGRATION_PROGRESS.zh-CN.md](./MIGRATION_PROGRESS.zh-CN.md)。

Last updated: `2026-03-14`

## Execution Summary (2026-03-14)

- Overall migration progress relative to the TypeScript baseline is **about 45% to 55%**, now closer to **~50%** than the earlier “well below half” snapshot.
- Phase status:
  - **Phase 1**: completed.
  - **Phase 2**: well underway, but not parity-complete.
  - **Phase 3**: baseline implemented in Rust `analyze`, but parity is still incomplete.
  - **Phase 4**: started at a lexical baseline, but semantic/hybrid parity is still missing.
  - **Phase 5**: native baseline exists, but parity remains incomplete.
  - **Phase 6**: not complete.

## Scope and Baseline

This document is a TS-baseline audit of the core Rust CLI surface and parity-critical runtime capabilities:

- Baseline (target): `GitNexus/gitnexus/src` (TypeScript implementation)
- Current Rust implementation: `src`
- Audit scope: Rust CLI entrypoints under `src` and parity-critical runtime behavior, not every TS helper entrypoint
- Evidence source: GitNexus MCP (`query/context/cypher`) + direct code inspection in `src` and `GitNexus/gitnexus/src`

### Audit Notes (2026-03-14)

- `gitnexus-rs` MCP index is current for this audit.
- Upstream `GitNexus` MCP index is `2` commits behind HEAD, so TS baseline claims below are based on MCP + filesystem inspection rather than MCP alone.
- TS helper entrypoints `ai-context`, `lazy-action`, and `tool` are outside the currently tracked migration surface because they do not have corresponding Rust command entrypoints under `src/commands`.

### Evidence Snapshot (2026-03-14)

- TS ingestion module size: `18` files (`GitNexus/gitnexus/src/core/ingestion/**`)
- Rust ingestion module size: `7` files (`src/ingestion/{mod,scan,symbols,relations,summaries,graph,parser_loader}.rs`)
- TS embeddings module size: `5` files (`.../core/embeddings/**`)
- TS search module size: `2` files (`.../core/search/**`)
- TS wiki module size: `5` files (`.../core/wiki/**`)
- TS MCP module size: `8` files (`.../src/mcp/**`)
- TS server module size: `2` files (`.../src/server/**`)

## Current Rust Runtime Status (TS as the reference line)

### Command-level status

- Rust-native by default:  
  `setup`, `analyze`, `list`, `status`, `clean`, `wiki`, `augment`, `eval-server`
- Rust-first via delegate wrapper:  
  `query`, `context`, `impact`, `cypher`, `detect-changes`, `rename`, `mcp`, `serve`
- Outside the currently tracked migration surface:  
  `ai-context`, `lazy-action`, `tool`

Note:

- `query/context/impact/cypher/detect-changes/rename/mcp/serve` currently route through `run_ts_cli(...)` wrappers, but are intercepted by Rust native dispatch (`local_tools::try_run_native_tool`) first.
- `augment` and `eval-server` now default to Rust-native paths, with TS fallback preserved as explicit opt-in via:
  - `GITNEXUS_USE_TS_AUGMENT=1`
  - `GITNEXUS_USE_TS_EVAL_SERVER=1`
- For query-like tools, Rust path is Kuzu-bridge first (`local_kuzu`) with heuristic fallback, not full TS parity.
- Rust `/api/search` now adds a native Kuzu FTS path before falling back to the local `query` baseline.

### Capability-level status (more important than command names)

- TS `analyzeCommand` executes full staged pipeline including:
  - ingestion pipeline orchestration (`runPipelineFromRepo`)
  - Kuzu load/materialization (`loadGraphToKuzu`)
  - FTS build (`createFTSIndex`)
  - embeddings pipeline (`runEmbeddingPipeline`, cache reuse path)
- Rust `analyze::run` currently does:
  - `run_ingestion_pipeline`
  - `rebuild_from_ingestion` (Kuzu materialization + FTS build)
  - save `.gitnexus/graph.json` fallback cache + meta/report/register
  - prints that `--embeddings` is not implemented
- TS `wikiCommand` + `WikiGenerator` includes:
  - LLM config resolution, interactive setup, richer generation workflow, optional gist publish
- Rust `wiki::run` is a baseline static generator and explicitly marks `model/base-url/api-key/concurrency/gist` as not implemented
- TS `/api/search` uses FTS plus optional semantic/hybrid retrieval.
- Rust `/api/search` now tries native Kuzu FTS first and falls back to the local `query` baseline, but still does not reach TS hybrid/semantic parity.
- Rust MCP/HTTP serving exists natively, but resources/prompts/setup/staleness guidance remain simplified compared to TS.

## Phase Plan (TS baseline, list only NOT Rust-only content)

The following follows the project migration rule sequence (Phase 1 -> Phase 6).

### Phase 1: Storage + Git + CLI Primitives

Goal: Remove remaining TS runtime dependency at CLI primitive level.

Status: Completed (`2026-03-12`).

Completed items:

1. `augment` now has a Rust-native fast path (hook-safe stderr contract, graceful failure behavior).
2. `eval-server` now has a Rust-native HTTP server path (`/tool/:name`, `/health`, `/shutdown`, idle timeout).
3. TS fallback is preserved as explicit opt-in (no longer default runtime path):
   - `GITNEXUS_USE_TS_AUGMENT=1`
   - `GITNEXUS_USE_TS_EVAL_SERVER=1`

Steps:

1. Keep TS fallback flags for emergency rollback only.
2. Baseline unit coverage for `augment` output shape and `eval-server` formatter/hint behavior is now in place (`2026-03-12`).
3. Baseline endpoint/hook integration parity is now in place (`2026-03-12`):
   - `eval-server` real HTTP endpoint contract (`/health`, `/tool/:name`, `/shutdown`)
   - `augment` `stderr` / short-input no-op / backend-failure silent behavior
4. Remove fallback flags after Phase 6 parity confidence gates are met.

### Phase 2: Ingestion Pipeline Parity

Goal: Move from heuristic single-file pipeline to TS-equivalent ingestion depth.

Remaining NOT Rust-only items:

1. Tree-sitter parser-loader capability parity (language availability and parser management).
2. Chunked parse pipeline parity is baseline-implemented:
   - Done baseline: bounded chunk planner (`max bytes` + `max files`) with stage-wise chunk execution for symbols/imports/calls/heritage.
   - Done baseline: progress callback contract per stage/chunk (`Symbols` / `Imports` / `Calls` / `Heritage`).
   - Done baseline: worker-pool style parallel chunk execution (parallelism derived from host CPU availability).
   - Done baseline: bounded worker-result backpressure (`sync_channel`) and fail-fast dispatch stop on first chunk error.
   - Remaining gap: TS-equivalent per-worker sub-batch timeout/retry tuning is not yet ported.
3. Import resolution context parity is now baseline-implemented for non-Rust runtime paths:
   - TypeScript/JavaScript: `tsconfig` `paths` + `baseUrl` alias/absolute resolution and suffix-based module-context fallback.
   - Go: `go.mod` module-path-based local package import resolution.
   - PHP: `composer.json` `autoload.psr-4` namespace-to-path resolution.
   - C#: `.csproj` `RootNamespace`-based using-namespace resolution.
   - Remaining gap: richer parser-backed import semantics (multi-config/workspace edge cases) still not TS-equivalent.
4. Route extraction and richer call/heritage extraction parity is partial:
   - Done baseline: Laravel `routes/*.php` to controller-method `CALLS` extraction (`Route::...([Controller::class, 'method'])`, `resource`, `apiResource`).
   - Done baseline: TypeScript/JavaScript route-like files now emit route-derived `CALLS` edges from file nodes to resolved handlers for common method patterns (`.get/.post/.put/.patch/.delete/.options/.head/.all/.use`) and common handler forms (identifier/member/array/wrapper arguments).
   - Remaining gap: deeper parser-backed framework semantics (multi-line chained routers, advanced decorators/macros, framework-specific conventions) is still not TS-equivalent.
5. Community/process processors parity (quality and metadata depth).

Steps:

1. Split `src/ingestion/mod.rs` into processors aligned with TS pipeline stages.
   - Baseline split-in-progress (`2026-03-12`): `scan`, `symbols`, `relations`, `summaries`, `graph`, and `parser_loader` were extracted into:
     - `src/ingestion/scan.rs`
     - `src/ingestion/symbols.rs`
     - `src/ingestion/relations.rs`
     - `src/ingestion/summaries.rs`
     - `src/ingestion/graph.rs`
     - `src/ingestion/parser_loader.rs`
   - `relations.rs` now owns import/call/heritage helper parsing functions (no longer cross-calling helper impls in `mod.rs`).
   - A baseline language capability matrix is introduced via `parser_loader` (call/heritage extraction remains TS/JS-only; import extraction baseline now extends to Go/PHP/C#).
   - `run_ingestion_pipeline` behavior is unchanged (all tests passing) while modular boundaries are introduced.
2. Introduce parser-loader abstraction and language capability matrix.
3. Add chunked parsing with bounded memory budget and progress callbacks.
   - Baseline completed (`2026-03-12`): `run_ingestion_pipeline_with_progress` now performs stage-wise chunked parsing and emits per-chunk progress events.
   - Baseline completed (`2026-03-12`): stage-wise chunk processing now executes via a worker-pool model.
   - Baseline completed (`2026-03-12`): worker result collection now uses bounded backpressure and stops scheduling new chunks after the first parse error.
   - Import parsing in chunk mode preserves full-repo resolution context via `parse_imports_for_sources(all_files, source_files)` to avoid cross-chunk misses.
4. Port remaining advanced call/heritage processing and deeper framework-specific route extraction.
5. Port community/process detection with deterministic output schema.

### Phase 3: Graph Persistence (Kuzu + Schema + Loaders)

Goal: Make Rust `analyze` produce TS-grade query-ready graph artifacts.

Status: Baseline implemented, parity incomplete (`2026-03-14`).

Remaining NOT Rust-only items:

1. Robust incremental reload/update behavior parity is missing.
2. Kuzu schema/loader fidelity and wider metadata coverage are still incomplete.
3. Analyze-time embeddings handoff is still missing (`--embeddings` is not implemented in Rust preview).

Steps:

1. Baseline completed (`2026-03-14`): Rust `analyze` now materializes Kuzu from ingestion, writes `.gitnexus/kuzu`, and persists `.gitnexus/graph.json` for fallback runtimes.
2. Baseline completed (`2026-03-14`): Rust `analyze` now attempts FTS extension load/install and builds per-table FTS indexes.
3. Remaining: align Kuzu schema and loader behavior more tightly to the TS baseline.
4. Remaining: add integrity checks and incremental reload/update semantics beyond the current safe full-rebuild path.

### Phase 4: Search + Embeddings

Goal: Reach TS retrieval depth and semantic quality.

Remaining NOT Rust-only items:

1. Embedding generation pipeline (including cache reuse) is missing in Rust.
2. Hybrid retrieval parity (lexical + semantic fusion) is incomplete.
3. `/api/search` behavior parity is incomplete; Rust currently has a lexical FTS baseline plus local `query` fallback, not TS hybrid ranking.

Steps:

1. Baseline completed (`2026-03-14`): Rust `/api/search` now attempts native Kuzu FTS before falling back to the local `query` baseline.
2. Implement embedding pipeline and storage format parity.
3. Add hybrid ranking strategy with reproducible scoring.
4. Align API output schema and ranking semantics with TS.
5. Add recall/precision regression tests for representative queries.

### Phase 5: MCP + HTTP Serving

Goal: Full runtime parity for tool/resource/prompt/server behavior.

Remaining NOT Rust-only items:

1. MCP resources/prompts/setup guidance parity is still simplified compared to TS.
2. Compatibility hardening for `eval-server` output contract across eval clients is incomplete.
3. Compatibility hardening for `augment` hook output contract across host tools is incomplete.

Steps:

1. Complete MCP resource/prompt parity (including setup and staleness hints).
2. Expand `eval-server` compatibility tests from baseline endpoint contract to multi-client edge cases.
3. Expand `augment` compatibility tests from baseline hook contract to host-specific integration matrix.
4. Add compatibility tests for Cursor/Claude/OpenCode integrations.

### Phase 6: Parity Validation and Rollout

Goal: Prevent silent drift from TS behavior after migration.

Remaining NOT Rust-only items:

1. Systematic parity test matrix is incomplete.
2. Golden outputs across major commands are incomplete.
3. Performance/memory guardrails across large repos are incomplete.

Steps:

1. Build command-level golden tests (`analyze/query/context/impact/cypher/wiki/serve/mcp`).
2. Build integration fixtures for known tricky repos/languages.
3. Define SLOs (latency/memory/index size) and gate in CI.
4. Add phased cutover plan: Rust default -> TS fallback optional -> TS removal.

## Recently Updated Functionality (vs older snapshot)

1. Rust native tool runtime path exists for `query/context/impact/cypher/detect-changes/rename/mcp/serve` through local tool dispatcher.
2. Rust query/context/impact/cypher have Kuzu bridge path with heuristic fallback path.
3. Rust `impact` already exposes relation filters and confidence threshold controls (`relation_types`, `min_confidence`) in CLI/MCP surface.
4. Rust-native `augment` and `eval-server` paths are now the default runtime path, with TS fallback gated by:
   - `GITNEXUS_USE_TS_AUGMENT=1`
   - `GITNEXUS_USE_TS_EVAL_SERVER=1`
5. Phase 1 baseline integration contract tests were added for `eval-server` HTTP endpoints and `augment` hook behavior.
6. Phase 2 baseline module split expanded for ingestion (`scan` + `symbols` + `relations` + `summaries` + `graph` + `parser_loader`) with behavior-preserving wrappers and a language capability matrix baseline.
7. `relations::parse_imports` includes advanced import resolution baseline:
   - `tsconfig` alias resolution (`compilerOptions.paths`)
   - `baseUrl` absolute specifier resolution
   - suffix-index module-context fallback for non-relative path-like imports
   - `go.mod` module-path local package resolution (Go)
   - `composer.json` `autoload.psr-4` namespace resolution (PHP)
   - `.csproj` `RootNamespace` namespace resolution (C#)
   - relation capability split: Go/PHP/C# are import-only for relations (call/heritage still TS/JS-only)
   - covered by ingestion tests for alias/baseUrl/module-context + Go/PHP/C# namespace/module cases
8. Phase 2 ingestion execution now has chunk/progress baseline:
   - bounded chunk planner (`PARSE_CHUNK_MAX_BYTES`, `PARSE_CHUNK_MAX_FILES`)
   - worker-pool parallel chunk processing (`symbols` / `imports` / `calls` / `heritage`)
   - public progress callback path (`run_ingestion_pipeline_with_progress`)
   - chunked import parsing keeps global import resolution context and is covered by regression tests.
9. Route extraction baseline is started in Rust calls pipeline:
   - Laravel route files (`routes/*.php`) now emit route-derived `CALLS` edges from file nodes to resolved controller methods.
   - covered by ingestion contract test for `Route::get(..., [Controller::class, 'method'])`.
10. TypeScript/JavaScript route-derived `CALLS` baseline is now added:
   - route-like files now emit `CALLS` edges from `File:<route file>` to resolved handlers for common route-method APIs (`.get/.post/.put/.patch/.delete/.options/.head/.all/.use`).
   - baseline handler form coverage includes identifier/member/array/wrapper-call arguments.
   - covered by ingestion contract test for route file handler resolution (`src/routes.ts` -> imported handlers).
11. Phase 2 chunk worker scheduling/backpressure baseline was hardened:
    - bounded worker-result channel in chunk execution to reduce peak queued memory under heavy extraction output.
    - fail-fast dispatch stop after first chunk error to avoid unnecessary extra chunk scheduling.
    - covered by ingestion unit regression for chunk-stage error path stability.
12. Phase 3 graph persistence baseline is now in the Rust analyze path:
    - `analyze` rebuilds `.gitnexus/kuzu` from ingestion and persists `.gitnexus/graph.json`.
    - `analyze` emits Kuzu/FTS summary output and only reports "Already up to date" when query-ready artifacts already exist for the current commit.
13. Rust server search now has a native lexical baseline:
    - `/api/search` tries Kuzu FTS first and falls back to the local `query` baseline when Kuzu/FTS is unavailable.

## Better Suggestions (for faster and safer migration)

1. Remove ambiguity in command ownership: replace `run_ts_cli` wrappers with explicit Rust command entrypoints where Rust is already the default path.
2. Keep TS fallback flags only as rollback switches, and schedule their removal after Phase 6 parity gates pass.
3. Treat `analyze` as the critical path: without analyze-time Kuzu/FTS/embeddings parity, advanced Rust tools will keep diverging from TS quality.
4. Add a parity dashboard in repo (phase -> command -> capability -> test status) and enforce updates in PR checklist.

# add-repo `--from-source` Design

**Date:** 2026-04-01

## Goal

Extend `gitn add-repo` with `--from-source=<remote_repo>` so the command can clone a remote repository into a local target directory and then add that directory to `repos.json` using the existing mapping workflow.

This iteration focuses on the command behavior only. Multi-language help text completeness is explicitly out of scope for now.

## Context

Current `add-repo` behavior only accepts an existing local source directory:

```bash
gitn add-repo <source-dir> [dest-name]
```

The command resolves the local directory, validates that it exists, derives a default `dest-name` from the directory basename, updates `repos.json`, and optionally triggers `start` / `analyze` follow-up steps.

The new behavior must preserve that model as much as possible.

## Chosen Approach

Use a thin "source materialization" step before the existing `add-repo` workflow:

1. Parse a new optional `--from-source` argument.
2. If `--from-source` is present, treat the first positional argument as a local clone target directory.
3. Run `git clone <remote_repo> <target_dir>`.
4. After clone succeeds, treat `<target_dir>` exactly like the existing local `<source-dir>`.
5. Reuse the current `repos.json` write path, duplicate/conflict detection, and `--restart` / `--analyze` follow-up behavior unchanged.

This was chosen over a separate remote-only code path so the command keeps one consistent meaning after the clone step completes.

## CLI Semantics

### Existing mode

Without `--from-source`, behavior remains unchanged:

```bash
gitn add-repo <source-dir> [dest-name]
```

### Remote clone mode

With `--from-source`, the first positional argument changes meaning:

```bash
gitn add-repo <target-dir> [dest-name] --from-source <remote-repo>
gitn add-repo <target-dir> [dest-name] --from-source=<remote-repo>
```

- `<target-dir>` is the local clone destination.
- After clone succeeds, `<target-dir>` becomes the effective local source directory for the existing workflow.
- `dest-name` remains the second positional argument.
- `dest-name` does not need to appear before `--from-source`; option ordering should not matter.

Equivalent examples:

```bash
gitn add-repo ~/xx/abrowser abrowser --from-source="https://github.com/xx/agent-browser"
gitn add-repo ~/xx/abrowser --from-source="https://github.com/xx/agent-browser" abrowser
```

## Defaults

- When `dest-name` is omitted in remote clone mode, default it from the local target directory basename.
- This keeps the remote mode aligned with existing local mode, where the effective source directory basename determines the default destination name.

Example:

```bash
gitn add-repo ~/xx/abrowser --from-source="https://github.com/xx/agent-browser"
```

This clones into `~/xx/abrowser` and then behaves as if the user ran:

```bash
gitn add-repo ~/xx/abrowser abrowser
```

## Error Handling Rules

### Argument validation

- `--from-source` must support both `--from-source=<remote_repo>` and `--from-source <remote_repo>`.
- If `--from-source` is present but the remote value is missing or empty, fail with a usage error.
- If `--from-source` is present but the first positional argument is missing, fail with a usage error because `<target-dir>` is required.

### Target directory behavior

- In remote clone mode, if `<target-dir>` already exists, fail immediately.
- Do not attempt to inspect whether the directory is a git repository.
- Do not attempt to reuse, overwrite, fetch, or pull.

### Clone failures

- If `git clone` fails for any reason, stop immediately.
- Do not write `repos.json`.
- Do not trigger `--restart`, `--rebuild`, or `--analyze`.

### Post-clone conflicts

- If clone succeeds but the existing duplicate/conflict rules reject the mapping, preserve the cloned local directory.
- Do not attempt automatic cleanup or rollback of the clone target.

## `repos.json` Behavior

`repos.json` remains unchanged in shape and meaning:

```json
{
  "repos": [
    ["/abs/host/path", "container_dir_name"]
  ]
}
```

In remote clone mode, the stored host path is the absolute path of the local clone target directory.

## Follow-up Behavior

`--restart`, `--rebuild`, and `--analyze` keep their current meaning.

- They run only after clone succeeds and `repos.json` is updated successfully.
- If clone fails or the mapping update fails, no follow-up action runs.

## Internal Implementation Design

The implementation should stay inside `packages/cli/src/commands/add_repo.zig` and avoid changing the downstream behavior of `start` or `analyze`.

Recommended structure:

1. Extend argument parsing with `from_source_remote: ?[]const u8`.
2. Add a small helper responsible for remote source preparation:
   - validate remote value
   - resolve `<target-dir>` to an absolute path
   - reject existing target directories
   - run `git clone`
   - return the effective local source path on success
3. Reuse the existing path normalization, default `dest-name`, duplicate detection, `repos.json` write, and follow-up logic after that helper returns.

This keeps the new capability narrow and avoids duplicating the main add-repo workflow.

## User-Facing Text Changes

Do not expand multi-language support as part of this change.

- The implementation may keep existing help/i18n coverage as-is for now.
- If a minimal help update is needed to avoid a completely hidden option, keep it narrow and do not treat full zh/en parity as a release requirement for this task.
- Shell completion updates are optional in this iteration and should not force multi-language text work.

## Testing Scope

Testing should focus on deterministic parsing and flow control, not real network access.

### Must cover

- `--from-source=<url>` parsing
- `--from-source <url>` parsing
- `dest-name` accepted either before or after `--from-source`
- default `dest-name` derived from local target directory basename
- remote mode rejects an already-existing target directory
- clone failure does not write `repos.json`
- clone success followed by a mapping conflict preserves the cloned directory while rejecting the mapping

### Testing strategy

- Prefer Zig unit tests around argument parsing and helper logic.
- Wrap the `git clone` invocation so tests can inject a fake runner instead of depending on actual git network operations.

## Non-Goals

- Full multi-language help and copy parity
- No automatic cleanup of clone targets on later validation failure
- No reuse of existing directories
- No implicit `fetch`, `pull`, or update behavior
- No changes to `repos.json` schema
- No new standalone subcommand for remote repositories

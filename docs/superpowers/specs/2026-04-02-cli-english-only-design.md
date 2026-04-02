# CLI English-Only Design

**Date:** 2026-04-02

## Goal

Remove multilingual behavior from `packages/cli` and keep English as the only supported UI language.

## Scope

This change removes language selection and translation branching from the CLI entrypoint, built-in commands, interactive prompts, and bundled shell completion output.

## User-Facing Behavior

- The CLI always renders English help text and prompt text.
- Global language flags `--lang`, `--zh`, and `--en` are no longer supported.
- Chinese command aliases such as `帮助` and `诊断` are no longer recognized.
- The zsh completion script no longer advertises language options or Chinese aliases.

## Implementation Design

### CLI Entrypoint

`packages/cli/src/main.zig` will stop parsing global language options and will classify only the English command names. Main help output will use a single embedded English template.

### Built-In Commands

The built-in command modules under `packages/cli/src/commands/` will embed only their English help templates. Command help rendering will no longer branch on environment variables or selected language state.

### Interactive Prompts

Interactive text in `packages/cli/src/commands/analyze.zig` and `packages/cli/src/container_runtime.zig` will be converted to fixed English strings. The shared language enum and language detection logic will be removed or reduced to the minimal shared helper surface still needed after the refactor.

### Completion Output

`packages/cli/src/completions/gitn.zsh` will remove language detection, global language option completions, and Chinese command alias completions. The completion script will describe only the English command surface.

### Translation Assets

The Chinese help template files in `packages/cli/src/i18n/` will be deleted. English templates remain as the single source of help text for the CLI.

## Testing Strategy

The change will follow TDD:

- Add a failing Zig test that proves global language flags are no longer parsed as CLI-wide options.
- Add a failing Zig test that proves Chinese command aliases are no longer classified as built-in commands.
- Add a failing Zig test that proves the emitted zsh completion script no longer contains `--lang`, `--zh`, `--en`, `帮助`, or `诊断`.
- Run focused Zig tests for the changed CLI modules.
- Run a CLI build to verify the refactor still compiles end-to-end.

## Non-Goals

- No compatibility shim for old language flags.
- No changes to non-CLI packages.
- No rewrite of unrelated CLI behavior outside language removal.

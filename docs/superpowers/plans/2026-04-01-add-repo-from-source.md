# Add Repo From Source Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add `--from-source` to `gitn add-repo` so a remote repository can be cloned into a local target directory before the existing mapping workflow runs.

**Architecture:** Keep `add-repo` as one command path. Extract argument parsing into a pure helper, add a narrow remote-source preparation helper with an injectable clone runner for tests, then reuse the current mapping, `repos.json` write, and follow-up logic without changing their semantics.

**Tech Stack:** Zig 0.15.2, `std.process.Child`, `std.testing`, existing CLI command structure in `packages/cli/src/commands/add_repo.zig`

---

## File Structure

- Modify: `packages/cli/src/commands/add_repo.zig`
  Responsibility: argument parsing, remote clone preparation, integration into `runWithArgs`, and all unit tests for this feature.
- Do not modify in this iteration: `packages/cli/src/i18n/*.txt`, `packages/cli/src/completions/gitn.zsh`
  Responsibility: multi-language help and completion parity are explicitly out of scope for this pass.

### Task 1: Extract `add-repo` Argument Parsing

**Files:**
- Modify: `packages/cli/src/commands/add_repo.zig`
- Test: `packages/cli/src/commands/add_repo.zig`

- [ ] **Step 1: Write the failing parser tests**

```zig
test "add_repo parse supports inline from-source syntax" {
    const testing = std.testing;
    const parsed = try parseAddRepoArgs(&.{
        "gitn add-repo",
        "~/xx/abrowser",
        "--from-source=https://github.com/xx/agent-browser",
    });

    try testing.expectEqualStrings("~/xx/abrowser", parsed.src_raw.?);
    try testing.expect(parsed.dest_name_arg == null);
    try testing.expectEqualStrings(
        "https://github.com/xx/agent-browser",
        parsed.from_source_remote.?,
    );
}

test "add_repo parse keeps dest-name as second positional after from-source" {
    const testing = std.testing;
    const parsed = try parseAddRepoArgs(&.{
        "gitn add-repo",
        "~/xx/abrowser",
        "--from-source",
        "https://github.com/xx/agent-browser",
        "abrowser",
    });

    try testing.expectEqualStrings("~/xx/abrowser", parsed.src_raw.?);
    try testing.expectEqualStrings("abrowser", parsed.dest_name_arg.?);
    try testing.expectEqualStrings(
        "https://github.com/xx/agent-browser",
        parsed.from_source_remote.?,
    );
}
```

- [ ] **Step 2: Run parser tests to verify they fail**

Run: `mise x -- zig test packages/cli/src/main.zig --test-filter "add_repo parse"`
Expected: FAIL with a compile error about missing `parseAddRepoArgs` or missing `from_source_remote` support.

- [ ] **Step 3: Write the minimal parser implementation**

```zig
const ParsedAddRepoArgs = struct {
    src_raw: ?[]const u8 = null,
    dest_name_arg: ?[]const u8 = null,
    restart_after: bool = false,
    analyze_after: bool = false,
    container_selector: ?[]const u8 = null,
    repos_override: ?[]const u8 = null,
    registry_override: ?[]const u8 = null,
    from_source_remote: ?[]const u8 = null,
    help_requested: bool = false,
};

fn parseAddRepoArgs(args: []const []const u8) !ParsedAddRepoArgs {
    var parsed = ParsedAddRepoArgs{};
    var i: usize = 1;

    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "-h") or std.mem.eql(u8, arg, "--help")) {
            parsed.help_requested = true;
            continue;
        }
        if (std.mem.eql(u8, arg, "--restart") or std.mem.eql(u8, arg, "--rebuild")) {
            parsed.restart_after = true;
            continue;
        }
        if (std.mem.eql(u8, arg, "--analyze")) {
            parsed.analyze_after = true;
            parsed.restart_after = true;
            continue;
        }
        if (std.mem.eql(u8, arg, "-c") or std.mem.eql(u8, arg, "--container")) {
            i += 1;
            if (i >= args.len) return error.InvalidArguments;
            const selector = std.mem.trim(u8, args[i], " \t\r\n");
            if (selector.len == 0) return error.InvalidArguments;
            parsed.container_selector = selector;
            continue;
        }
        if (std.mem.eql(u8, arg, "--repos")) {
            i += 1;
            if (i >= args.len) return error.InvalidArguments;
            const repos_path = std.mem.trim(u8, args[i], " \t\r\n");
            if (repos_path.len == 0) return error.InvalidArguments;
            parsed.repos_override = repos_path;
            continue;
        }
        if (std.mem.eql(u8, arg, "--registry")) {
            i += 1;
            if (i >= args.len) return error.InvalidArguments;
            const registry_path = std.mem.trim(u8, args[i], " \t\r\n");
            if (registry_path.len == 0) return error.InvalidArguments;
            parsed.registry_override = registry_path;
            continue;
        }
        if (std.mem.eql(u8, arg, "--from-source")) {
            i += 1;
            if (i >= args.len) return error.InvalidArguments;
            const remote = std.mem.trim(u8, args[i], " \t\r\n");
            if (remote.len == 0) return error.InvalidArguments;
            parsed.from_source_remote = remote;
            continue;
        }
        if (std.mem.startsWith(u8, arg, "--from-source=")) {
            const remote = std.mem.trim(u8, arg["--from-source=".len..], " \t\r\n");
            if (remote.len == 0) return error.InvalidArguments;
            parsed.from_source_remote = remote;
            continue;
        }
        if (std.mem.startsWith(u8, arg, "-")) {
            return error.InvalidArguments;
        }

        if (parsed.src_raw == null) {
            parsed.src_raw = arg;
            continue;
        }
        if (parsed.dest_name_arg == null) {
            parsed.dest_name_arg = arg;
            continue;
        }
        return error.InvalidArguments;
    }

    return parsed;
}
```

- [ ] **Step 4: Run parser tests to verify they pass**

Run: `mise x -- zig test packages/cli/src/main.zig --test-filter "add_repo parse"`
Expected: `All 2 tests passed.`

- [ ] **Step 5: Commit**

```bash
git add packages/cli/src/commands/add_repo.zig
git commit -m "refactor: extract add-repo argument parsing"
```

### Task 2: Add Remote Source Preparation Helper

**Files:**
- Modify: `packages/cli/src/commands/add_repo.zig`
- Test: `packages/cli/src/commands/add_repo.zig`

- [ ] **Step 1: Write the failing remote preparation tests**

```zig
var test_clone_call_count: usize = 0;
var test_clone_remote: ?[]const u8 = null;

fn fakeCloneCreatesDir(_: Allocator, remote: []const u8, target_abs: []const u8) !void {
    test_clone_call_count += 1;
    test_clone_remote = remote;
    try std.fs.makeDirAbsolute(target_abs);
}

fn fakeCloneFails(_: Allocator, _: []const u8, _: []const u8) !void {
    return error.GitCloneFailed;
}

test "add_repo ensureRemoteSourceReady rejects existing target directory" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);
    try std.fs.makeDirAbsolute(target_abs);

    try testing.expectError(
        error.TargetAlreadyExists,
        ensureRemoteSourceReady(testing.allocator, root_abs, "", target_abs, "https://example.com/repo.git"),
    );
}

test "add_repo ensureRemoteSourceReady clones into missing target directory" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);

    const original_runner = clone_runner;
    clone_runner = fakeCloneCreatesDir;
    defer clone_runner = original_runner;

    test_clone_call_count = 0;
    test_clone_remote = null;

    const prepared = try ensureRemoteSourceReady(
        testing.allocator,
        root_abs,
        "",
        target_abs,
        "https://example.com/repo.git",
    );
    defer testing.allocator.free(prepared);

    try testing.expectEqual(@as(usize, 1), test_clone_call_count);
    try testing.expectEqualStrings("https://example.com/repo.git", test_clone_remote.?);
    try testing.expectEqualStrings(target_abs, prepared);
}
```

- [ ] **Step 2: Run remote preparation tests to verify they fail**

Run: `mise x -- zig test packages/cli/src/main.zig --test-filter "ensureRemoteSourceReady"`
Expected: FAIL with a compile error about missing `clone_runner` or missing `ensureRemoteSourceReady`.

- [ ] **Step 3: Write the minimal remote preparation implementation**

```zig
const CloneRunner = *const fn (allocator: Allocator, remote: []const u8, target_abs: []const u8) anyerror!void;

fn defaultCloneRunner(allocator: Allocator, remote: []const u8, target_abs: []const u8) !void {
    const result = std.process.Child.run(.{
        .allocator = allocator,
        .argv = &.{ "git", "clone", remote, target_abs },
    }) catch |err| switch (err) {
        error.FileNotFound => {
            std.debug.print("Error: git executable not found on PATH\n", .{});
            return error.GitCloneFailed;
        },
        else => return err,
    };

    if (!isExitedZero(result.term)) {
        const stderr_text = std.mem.trim(u8, result.stderr, " \t\r\n");
        if (stderr_text.len != 0) {
            std.debug.print("Error: git clone failed: {s}\n", .{stderr_text});
        } else {
            std.debug.print("Error: git clone failed for {s}\n", .{remote});
        }
        return error.GitCloneFailed;
    }
}

var clone_runner: CloneRunner = defaultCloneRunner;

fn ensureRemoteSourceReady(
    allocator: Allocator,
    cwd_abs: []const u8,
    home_dir: []const u8,
    src_raw: []const u8,
    remote: []const u8,
) ![]const u8 {
    const target_abs = try normalizeSourcePath(allocator, src_raw, home_dir, cwd_abs);

    if (std.fs.openDirAbsolute(target_abs, .{})) |dir| {
        dir.close();
        std.debug.print("Error: clone target already exists: {s}\n", .{target_abs});
        return error.TargetAlreadyExists;
    } else |err| switch (err) {
        error.FileNotFound => {},
        error.NotDir => {
            std.debug.print("Error: clone target already exists: {s}\n", .{target_abs});
            return error.TargetAlreadyExists;
        },
        else => return err,
    }

    try clone_runner(allocator, remote, target_abs);
    return target_abs;
}
```

- [ ] **Step 4: Run remote preparation tests to verify they pass**

Run: `mise x -- zig test packages/cli/src/main.zig --test-filter "ensureRemoteSourceReady"`
Expected: `All 2 tests passed.`

- [ ] **Step 5: Commit**

```bash
git add packages/cli/src/commands/add_repo.zig
git commit -m "feat: prepare remote sources for add-repo"
```

### Task 3: Integrate Remote Mode Into `runWithArgs`

**Files:**
- Modify: `packages/cli/src/commands/add_repo.zig`
- Test: `packages/cli/src/commands/add_repo.zig`

- [ ] **Step 1: Write the failing integration tests**

```zig
fn fakeCloneCreatesRepoDir(_: Allocator, _: []const u8, target_abs: []const u8) !void {
    try std.fs.makeDirAbsolute(target_abs);
}

test "add_repo remote mode writes repos using target basename by default" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);

    const repos_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "repos.json" });
    defer testing.allocator.free(repos_abs);

    const original_runner = clone_runner;
    clone_runner = fakeCloneCreatesRepoDir;
    defer clone_runner = original_runner;

    const exit_code = try runWithArgs(testing.allocator, &.{
        "gitn add-repo",
        target_abs,
        "--from-source=https://github.com/xx/agent-browser",
        "--repos",
        repos_abs,
    });

    try testing.expectEqual(@as(u8, 0), exit_code);

    const repos_content = try std.fs.cwd().readFileAlloc(testing.allocator, repos_abs, 4096);
    defer testing.allocator.free(repos_content);

    try testing.expect(std.mem.containsAtLeast(u8, repos_content, 1, target_abs));
    try testing.expect(std.mem.containsAtLeast(u8, repos_content, 1, "\"abrowser\""));
}

test "add_repo remote clone failure does not write repos.json" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);

    const repos_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "repos.json" });
    defer testing.allocator.free(repos_abs);

    const original_runner = clone_runner;
    clone_runner = fakeCloneFails;
    defer clone_runner = original_runner;

    const exit_code = try runWithArgs(testing.allocator, &.{
        "gitn add-repo",
        target_abs,
        "--from-source=https://github.com/xx/agent-browser",
        "--repos",
        repos_abs,
    });

    try testing.expectEqual(@as(u8, 1), exit_code);
    try testing.expectError(
        error.FileNotFound,
        std.fs.cwd().readFileAlloc(testing.allocator, repos_abs, 4096),
    );
}

test "add_repo remote conflict preserves cloned target directory" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const existing_source = try std.fs.path.join(testing.allocator, &.{ root_abs, "existing-src" });
    defer testing.allocator.free(existing_source);
    try std.fs.makeDirAbsolute(existing_source);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);

    const repos_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "repos.json" });
    defer testing.allocator.free(repos_abs);
    const seeded_repos = try std.fmt.allocPrint(
        testing.allocator,
        "{{\n  \"repos\": [[\"{s}\", \"abrowser\"]]\n}}\n",
        .{existing_source},
    );
    defer testing.allocator.free(seeded_repos);

    var repos_file = try std.fs.createFileAbsolute(repos_abs, .{ .truncate = true });
    defer repos_file.close();
    try repos_file.writeAll(seeded_repos);

    const original_runner = clone_runner;
    clone_runner = fakeCloneCreatesRepoDir;
    defer clone_runner = original_runner;

    const exit_code = try runWithArgs(testing.allocator, &.{
        "gitn add-repo",
        target_abs,
        "--from-source=https://github.com/xx/agent-browser",
        "--repos",
        repos_abs,
    });

    try testing.expectEqual(@as(u8, 1), exit_code);

    var cloned_dir = try std.fs.openDirAbsolute(target_abs, .{});
    cloned_dir.close();

    const repos_content = try std.fs.cwd().readFileAlloc(testing.allocator, repos_abs, 4096);
    defer testing.allocator.free(repos_content);
    try testing.expect(std.mem.containsAtLeast(u8, repos_content, 1, existing_source));
    try testing.expect(!std.mem.containsAtLeast(u8, repos_content, 1, target_abs));
}
```

- [ ] **Step 2: Run integration tests to verify they fail**

Run: `mise x -- zig test packages/cli/src/main.zig --test-filter "add_repo remote"`
Expected: FAIL because `runWithArgs` still treats the first positional as an already-existing local source directory.

- [ ] **Step 3: Add the remote-aware source resolver**

```zig
fn resolveEffectiveSourcePath(
    allocator: Allocator,
    cwd_abs: []const u8,
    home_dir: []const u8,
    parsed: ParsedAddRepoArgs,
) ![]const u8 {
    const src_raw = parsed.src_raw orelse return error.InvalidArguments;

    if (parsed.from_source_remote) |remote| {
        return ensureRemoteSourceReady(allocator, cwd_abs, home_dir, src_raw, remote);
    }

    return std.fs.cwd().realpathAlloc(allocator, src_raw) catch {
        std.debug.print("Error: source directory does not exist: {s}\n", .{src_raw});
        return error.SourceDirMissing;
    };
}
```

- [ ] **Step 4: Replace the old top-of-function mutable locals with parsed fields**

```zig
const parsed = parseAddRepoArgs(args) catch {
    try printUsage(allocator, args[0]);
    return 1;
};

if (parsed.help_requested) {
    try printUsage(allocator, args[0]);
    return 0;
}
if (parsed.src_raw == null) {
    try printUsage(allocator, args[0]);
    return 1;
}

const container_selector = parsed.container_selector;
const repos_override = parsed.repos_override;
const registry_override = parsed.registry_override;
const restart_after = parsed.restart_after;
const analyze_after = parsed.analyze_after;
```

- [ ] **Step 5: Replace the old local-source resolution block with remote-aware source resolution**

```zig
const cwd_abs = try std.fs.cwd().realpathAlloc(allocator, ".");
const home_dir = try detectHomeDir(allocator);

const src_abs = resolveEffectiveSourcePath(allocator, cwd_abs, home_dir, parsed) catch |err| switch (err) {
    error.TargetAlreadyExists,
    error.GitCloneFailed,
    error.SourceDirMissing,
    => return 1,
    else => return err,
};

var source_dir = std.fs.openDirAbsolute(src_abs, .{}) catch {
    std.debug.print("Error: source path is not a directory: {s}\n", .{src_abs});
    return 1;
};
source_dir.close();

const dest_name = if (parsed.dest_name_arg) |dest_arg|
    dest_arg
else
    std.fs.path.basename(src_abs);
```

- [ ] **Step 6: Run the focused tests and the full CLI test entry**

Run: `mise x -- zig test packages/cli/src/main.zig --test-filter "add_repo remote"`
Expected: `All 3 tests passed.`

Run: `mise x -- zig test packages/cli/src/main.zig`
Expected: `All tests passed.`

- [ ] **Step 7: Commit**

```bash
git add packages/cli/src/commands/add_repo.zig
git commit -m "feat: support add-repo from remote source"
```

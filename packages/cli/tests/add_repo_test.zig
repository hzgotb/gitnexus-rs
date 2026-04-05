const std = @import("std");
const cli = @import("cli_main");
const add_repo = cli.testing.add_repo_module;

const Allocator = std.mem.Allocator;
const CloneRunner = add_repo.testing.CloneRunnerType;

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

test "add_repo parse supports inline from-source syntax" {
    const testing = std.testing;
    const parsed = try add_repo.testing.parseArgs(&.{
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
    const parsed = try add_repo.testing.parseArgs(&.{
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

test "add_repo default clone child inherits terminal stdio for visible output" {
    const testing = std.testing;
    var argv = [_][]const u8{
        "git",
        "clone",
        "https://example.com/repo.git",
        "/tmp/repo",
    };
    var child = std.process.Child.init(&argv, testing.allocator);

    add_repo.testing.configureCloneChildForTest(&child);

    try testing.expect(child.stdin_behavior == .Inherit);
    try testing.expect(child.stdout_behavior == .Inherit);
    try testing.expect(child.stderr_behavior == .Inherit);
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
        add_repo.testing.ensureRemoteSourceReadyForTest(
            testing.allocator,
            root_abs,
            "",
            target_abs,
            "https://example.com/repo.git",
        ),
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

    const original_runner = add_repo.testing.setCloneRunner(fakeCloneCreatesDir);
    defer _ = add_repo.testing.setCloneRunner(original_runner);

    test_clone_call_count = 0;
    test_clone_remote = null;

    const prepared = try add_repo.testing.ensureRemoteSourceReadyForTest(
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

test "add_repo local mode writes repos.json for source dir" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const source_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(source_abs);
    try std.fs.makeDirAbsolute(source_abs);

    const repos_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "repos.json" });
    defer testing.allocator.free(repos_abs);

    const exit_code = try add_repo.runWithArgs(arena.allocator(), &.{
        "gitn add-repo",
        source_abs,
        "--repos",
        repos_abs,
    });

    try testing.expectEqual(@as(u8, 0), exit_code);

    const repos_content = try std.fs.cwd().readFileAlloc(testing.allocator, repos_abs, 4096);
    defer testing.allocator.free(repos_content);

    try testing.expect(std.mem.containsAtLeast(u8, repos_content, 1, source_abs));
    try testing.expect(std.mem.containsAtLeast(u8, repos_content, 1, "\"abrowser\""));
}

test "add_repo remote clone failure does not write repos.json" {
    const testing = std.testing;
    var tmp = testing.tmpDir(.{});
    defer tmp.cleanup();
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);

    const repos_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "repos.json" });
    defer testing.allocator.free(repos_abs);

    const original_runner = add_repo.testing.setCloneRunner(fakeCloneFails);
    defer _ = add_repo.testing.setCloneRunner(original_runner);

    const exit_code = try add_repo.runWithArgs(arena.allocator(), &.{
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
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();

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

    const original_runner = add_repo.testing.setCloneRunner(fakeCloneCreatesDir);
    defer _ = add_repo.testing.setCloneRunner(original_runner);

    const exit_code = try add_repo.runWithArgs(arena.allocator(), &.{
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

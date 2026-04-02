const std = @import("std");
const analyze_cmd = @import("analyze.zig");
const container_group = @import("../container_group.zig");
const container_runtime = @import("../container_runtime.zig");
const i18n = @import("../i18n.zig");
const start_cmd = @import("start.zig");

const Allocator = std.mem.Allocator;
const JsonValue = std.json.Value;
const help_en = @embedFile("../i18n/add_repo_help.en.txt");
const help_zh = @embedFile("../i18n/add_repo_help.zh.txt");
const container_name_prefix = "gitnexus";

const Status = enum {
    append,
    same_src_ok,
    conflict_src,
    conflict_dest,
};

const RepoEntry = struct {
    src: []const u8 = "",
    dest: []const u8 = "",
};

const ExistingContainer = struct {
    id: []const u8,
    name: []const u8,
};

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

const CloneRunner = *const fn (allocator: Allocator, remote: []const u8, target_abs: []const u8) anyerror!void;

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
        if (std.mem.startsWith(u8, arg, "-")) return error.InvalidArguments;

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
    defer allocator.free(result.stdout);
    defer allocator.free(result.stderr);

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
    errdefer allocator.free(target_abs);

    if (std.fs.openDirAbsolute(target_abs, .{})) |opened_dir| {
        var dir = opened_dir;
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

fn printUsage(allocator: Allocator, exe_name: []const u8) !void {
    const lang = i18n.detectLangFromEnv(allocator);
    const tpl = switch (lang) {
        .zh => help_zh,
        .en => help_en,
    };
    try i18n.printHelpTemplate(allocator, tpl, exe_name);
}

fn isManagedContainerName(name: []const u8) bool {
    return std.mem.eql(u8, name, container_name_prefix) or std.mem.startsWith(u8, name, container_name_prefix ++ "-");
}

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn runDocker(allocator: Allocator, argv: []const []const u8) !std.process.Child.RunResult {
    return container_runtime.runDocker(allocator, argv);
}

fn resolveContainerSelectorToName(allocator: Allocator, selector: []const u8) ![]const u8 {
    const result = try runDocker(allocator, &.{
        "docker",
        "ps",
        "-a",
        "--format",
        "{{.ID}}\t{{.Names}}",
    });
    if (!isExitedZero(result.term)) {
        std.debug.print("Error: failed to list docker containers.\n", .{});
        return error.DockerPsFailed;
    }

    var containers = std.ArrayList(ExistingContainer).empty;
    var lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;

        var cols = std.mem.splitScalar(u8, line, '\t');
        const id = cols.next() orelse continue;
        const name = cols.next() orelse continue;
        if (!isManagedContainerName(name)) continue;

        try containers.append(allocator, .{
            .id = try allocator.dupe(u8, id),
            .name = try allocator.dupe(u8, name),
        });
    }

    var exact_name: ?[]const u8 = null;
    for (containers.items) |container| {
        if (std.mem.eql(u8, container.name, selector) or std.mem.eql(u8, container.id, selector)) {
            if (exact_name != null) return error.AmbiguousSelector;
            exact_name = container.name;
        }
    }
    if (exact_name) |name| return name;

    var prefix_name: ?[]const u8 = null;
    for (containers.items) |container| {
        if (std.mem.startsWith(u8, container.id, selector)) {
            if (prefix_name != null) return error.AmbiguousSelector;
            prefix_name = container.name;
        }
    }
    return prefix_name orelse error.ContainerNotFound;
}

fn getEnvVarOwnedOrEmpty(allocator: Allocator, name: []const u8) ![]const u8 {
    return std.process.getEnvVarOwned(allocator, name) catch |err| switch (err) {
        error.EnvironmentVariableNotFound => "",
        else => return err,
    };
}

fn detectHomeDir(allocator: Allocator) ![]const u8 {
    const home = try getEnvVarOwnedOrEmpty(allocator, "HOME");
    if (home.len != 0) return home;

    const user_profile = try getEnvVarOwnedOrEmpty(allocator, "USERPROFILE");
    if (user_profile.len != 0) return user_profile;

    return "";
}

fn normalizeSourcePath(
    allocator: Allocator,
    raw_src: []const u8,
    home_dir: []const u8,
    cwd_abs: []const u8,
) ![]const u8 {
    const trimmed = std.mem.trim(u8, raw_src, " \t\r\n");
    if (trimmed.len == 0) return "";

    var candidate: []const u8 = trimmed;
    if (home_dir.len != 0) {
        if (std.mem.eql(u8, trimmed, "~")) {
            candidate = home_dir;
        } else if (std.mem.startsWith(u8, trimmed, "~/") or std.mem.startsWith(u8, trimmed, "~\\")) {
            candidate = try std.fs.path.join(allocator, &.{ home_dir, trimmed[2..] });
        }
    }

    if (std.fs.path.isAbsolute(candidate)) {
        return try std.fs.path.resolve(allocator, &.{candidate});
    }
    return try std.fs.path.resolve(allocator, &.{ cwd_abs, candidate });
}

fn resolvePathFromCwd(allocator: Allocator, cwd_abs: []const u8, raw_path: []const u8) ![]const u8 {
    if (std.fs.path.isAbsolute(raw_path)) {
        return try std.fs.path.resolve(allocator, &.{raw_path});
    }
    return try std.fs.path.resolve(allocator, &.{ cwd_abs, raw_path });
}

fn firstStringField(obj: *const std.json.ObjectMap, keys: []const []const u8) []const u8 {
    for (keys) |key| {
        if (obj.get(key)) |value| {
            switch (value) {
                .string => |s| return s,
                else => {},
            }
        }
    }
    return "";
}

fn extractEntry(value: *const JsonValue) RepoEntry {
    switch (value.*) {
        .array => |arr| {
            var src: []const u8 = "";
            var dest: []const u8 = "";

            if (arr.items.len >= 1) {
                switch (arr.items[0]) {
                    .string => |s| src = s,
                    else => {},
                }
            }
            if (arr.items.len >= 2) {
                switch (arr.items[1]) {
                    .string => |s| dest = s,
                    else => {},
                }
            }

            return .{ .src = src, .dest = dest };
        },
        .object => |obj| {
            return .{
                .src = firstStringField(&obj, &.{ "src_abs", "src", "source", "SRC_ABS" }),
                .dest = firstStringField(&obj, &.{ "dest_name", "dest", "target", "DEST_NAME" }),
            };
        },
        else => return .{},
    }
}

fn makeRepoEntry(allocator: Allocator, src_abs: []const u8, dest_name: []const u8) !JsonValue {
    var arr = std.json.Array.init(allocator);
    try arr.append(.{ .string = try allocator.dupe(u8, src_abs) });
    try arr.append(.{ .string = try allocator.dupe(u8, dest_name) });
    return .{ .array = arr };
}

fn writeJsonAtomic(allocator: Allocator, root: JsonValue, repos_json_path: []const u8) !void {
    const dir_path = std.fs.path.dirname(repos_json_path) orelse ".";
    const file_name = std.fs.path.basename(repos_json_path);
    const tmp_name = try std.fmt.allocPrint(allocator, "{s}.tmp.{d}", .{
        file_name,
        std.time.milliTimestamp(),
    });

    var dir = if (std.fs.path.isAbsolute(repos_json_path))
        try std.fs.openDirAbsolute(dir_path, .{})
    else
        try std.fs.cwd().openDir(dir_path, .{});
    defer dir.close();
    errdefer dir.deleteFile(tmp_name) catch {};

    var tmp_file = try dir.createFile(tmp_name, .{ .truncate = true });
    defer tmp_file.close();

    var writer_buffer: [4096]u8 = undefined;
    var file_writer = tmp_file.writer(&writer_buffer);
    try std.json.Stringify.value(root, .{ .whitespace = .indent_2 }, &file_writer.interface);
    try file_writer.interface.writeAll("\n");
    try file_writer.interface.flush();

    dir.rename(tmp_name, file_name) catch |err| switch (err) {
        error.PathAlreadyExists => {
            try dir.deleteFile(file_name);
            try dir.rename(tmp_name, file_name);
        },
        else => return err,
    };
}

fn runStartFollowUp(
    allocator: Allocator,
    container_selector: ?[]const u8,
    registry_path: ?[]const u8,
    repos_path: ?[]const u8,
) !u8 {
    var forwarded = std.ArrayList([]const u8).empty;
    try forwarded.append(allocator, "gitn start");
    if (container_selector) |selector| {
        try forwarded.appendSlice(allocator, &.{ "--rebuild", selector });
    }
    if (registry_path) |path| {
        try forwarded.appendSlice(allocator, &.{ "--registry", path });
    }
    if (repos_path) |path| {
        try forwarded.appendSlice(allocator, &.{ "--repos", path });
    }
    return start_cmd.runWithArgs(allocator, forwarded.items);
}

fn runAnalyzeFollowUp(
    allocator: Allocator,
    container_selector: ?[]const u8,
    repo_name: []const u8,
) !u8 {
    var forwarded = std.ArrayList([]const u8).empty;
    try forwarded.append(allocator, "gitn analyze");
    if (container_selector) |selector| {
        try forwarded.appendSlice(allocator, &.{ "--container", selector });
    }
    try forwarded.appendSlice(allocator, &.{ "--repo", repo_name });
    return analyze_cmd.runWithArgs(allocator, forwarded.items);
}

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

pub fn runWithArgs(allocator: Allocator, args: []const []const u8) !u8 {
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

    const cwd_abs = try std.fs.cwd().realpathAlloc(allocator, ".");
    const home_dir = try detectHomeDir(allocator);
    const target_container_name = if (container_selector) |selector|
        resolveContainerSelectorToName(allocator, selector) catch |err| switch (err) {
            error.DockerPsFailed => return 1,
            error.ContainerNotFound => {
                std.debug.print("Error: target container not found: {s}\n", .{selector});
                return 1;
            },
            error.AmbiguousSelector => {
                std.debug.print("Error: ambiguous container selector: {s}\n", .{selector});
                return 1;
            },
            else => return err,
        }
    else
        null;
    const inferred_group_paths = if (target_container_name) |container_name|
        container_group.inspectContainerJsonPaths(allocator, container_name) catch container_group.JsonGroupPaths{}
    else
        container_group.JsonGroupPaths{};

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

    const dest_name: []const u8 = if (parsed.dest_name_arg) |dest_arg|
        dest_arg
    else
        std.fs.path.basename(src_abs);

    const repos_json_path = if (repos_override) |path|
        try resolvePathFromCwd(allocator, cwd_abs, path)
    else if (inferred_group_paths.repos_json_path.len != 0)
        inferred_group_paths.repos_json_path
    else
        try std.fs.path.join(allocator, &.{ cwd_abs, "repos.json" });
    const file_content = std.fs.cwd().readFileAlloc(allocator, repos_json_path, 16 * 1024 * 1024) catch |err| switch (err) {
        error.FileNotFound => null,
        else => return err,
    };

    var parsed_json: ?std.json.Parsed(JsonValue) = null;
    defer {
        if (parsed_json) |*p| p.deinit();
    }

    var root: JsonValue = undefined;
    if (file_content) |content| {
        defer allocator.free(content);

        parsed_json = std.json.parseFromSlice(JsonValue, allocator, content, .{
            .allocate = .alloc_always,
        }) catch {
            std.debug.print("Error: repos.json is not valid JSON\n", .{});
            return 1;
        };
        root = parsed_json.?.value;
    } else {
        root = .{ .object = std.json.ObjectMap.init(allocator) };
    }

    var root_obj: *std.json.ObjectMap = undefined;
    switch (root) {
        .object => |*obj| root_obj = obj,
        else => {
            std.debug.print("Error: repos.json root must be a JSON object\n", .{});
            return 1;
        },
    }

    var repos = std.json.Array.init(allocator);
    if (root_obj.get("repos")) |repos_value| {
        switch (repos_value) {
            .array => |arr| repos = arr,
            else => {},
        }
    }

    const new_src_norm = try normalizeSourcePath(allocator, src_abs, home_dir, cwd_abs);

    var status: Status = .append;
    var old_dest: []const u8 = "";
    var old_src: []const u8 = "";
    var same_src_index: ?usize = null;

    for (repos.items, 0..) |*entry, idx| {
        if (status != .append) break;

        const old_entry = extractEntry(entry);
        const old_src_norm = try normalizeSourcePath(allocator, old_entry.src, home_dir, cwd_abs);

        if (old_src_norm.len != 0 and std.mem.eql(u8, old_src_norm, new_src_norm)) {
            if (old_entry.dest.len == 0 or std.mem.eql(u8, old_entry.dest, dest_name)) {
                status = .same_src_ok;
                same_src_index = idx;
                old_dest = old_entry.dest;
            } else {
                status = .conflict_src;
                old_dest = old_entry.dest;
            }
            continue;
        }

        if (old_entry.dest.len != 0 and std.mem.eql(u8, old_entry.dest, dest_name) and !std.mem.eql(u8, old_src_norm, new_src_norm)) {
            status = .conflict_dest;
            old_src = old_entry.src;
        }
    }

    switch (status) {
        .conflict_src => {
            std.debug.print(
                "Error: source is already mapped to '{s}', cannot remap to '{s}'\n",
                .{ old_dest, dest_name },
            );
            return 1;
        },
        .conflict_dest => {
            std.debug.print(
                "Error: destination name '{s}' is already used by another source ({s})\n",
                .{ dest_name, old_src },
            );
            return 1;
        },
        else => {},
    }

    const new_entry = try makeRepoEntry(allocator, src_abs, dest_name);
    if (same_src_index) |idx| {
        repos.items[idx] = new_entry;
    } else {
        try repos.append(new_entry);
    }

    try root_obj.put("repos", .{ .array = repos });
    try writeJsonAtomic(allocator, root, repos_json_path);

    if (status == .same_src_ok) {
        if (std.mem.eql(u8, old_dest, dest_name)) {
            std.debug.print("Info: same mapping already exists, skipped duplicate append.\n", .{});
        } else {
            std.debug.print("Info: source already existed, destination name was filled/updated.\n", .{});
        }
    } else {
        std.debug.print("OK: {s} updated: {s} -> {s}\n", .{ repos_json_path, src_abs, dest_name });
    }

    if (!restart_after and !analyze_after) return 0;

    const followup_registry_path = if (registry_override) |path|
        try resolvePathFromCwd(allocator, cwd_abs, path)
    else if (inferred_group_paths.registry_json_path.len != 0)
        inferred_group_paths.registry_json_path
    else
        null;

    const start_exit_code = try runStartFollowUp(
        allocator,
        target_container_name,
        followup_registry_path,
        repos_json_path,
    );
    if (start_exit_code != 0) return start_exit_code;
    if (!analyze_after) return 0;

    return try runAnalyzeFollowUp(allocator, target_container_name, dest_name);
}

fn run(allocator: Allocator) !u8 {
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);
    return runWithArgs(allocator, args);
}

pub fn main() void {
    var arena = std.heap.ArenaAllocator.init(std.heap.page_allocator);
    defer arena.deinit();

    const exit_code = run(arena.allocator()) catch |err| {
        std.debug.print("Error: {s}\n", .{@errorName(err)});
        std.process.exit(1);
    };
    std.process.exit(exit_code);
}

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

test "add_repo remote mode writes repos using target basename by default" {
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

    const original_runner = clone_runner;
    clone_runner = fakeCloneCreatesDir;
    defer clone_runner = original_runner;

    const exit_code = try runWithArgs(arena.allocator(), &.{
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
    var arena = std.heap.ArenaAllocator.init(testing.allocator);
    defer arena.deinit();

    const root_abs = try tmp.dir.realpathAlloc(testing.allocator, ".");
    defer testing.allocator.free(root_abs);

    const target_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "abrowser" });
    defer testing.allocator.free(target_abs);

    const repos_abs = try std.fs.path.join(testing.allocator, &.{ root_abs, "repos.json" });
    defer testing.allocator.free(repos_abs);

    const original_runner = clone_runner;
    clone_runner = fakeCloneFails;
    defer clone_runner = original_runner;

    const exit_code = try runWithArgs(arena.allocator(), &.{
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

    const original_runner = clone_runner;
    clone_runner = fakeCloneCreatesDir;
    defer clone_runner = original_runner;

    const exit_code = try runWithArgs(arena.allocator(), &.{
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

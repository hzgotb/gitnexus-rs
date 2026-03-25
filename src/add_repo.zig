const std = @import("std");

const Allocator = std.mem.Allocator;
const JsonValue = std.json.Value;

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

fn printUsage(exe_name: []const u8) void {
    std.debug.print(
        \\Usage:
        \\  {s} <source-dir> [dest-name]
        \\
        \\Options:
        \\  -h, --help    Show this help message
        \\
    ,
        .{exe_name},
    );
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
    const tmp_path = try std.fmt.allocPrint(allocator, "{s}.tmp.{d}", .{
        repos_json_path,
        std.time.milliTimestamp(),
    });

    errdefer std.fs.cwd().deleteFile(tmp_path) catch {};

    var tmp_file = try std.fs.cwd().createFile(tmp_path, .{ .truncate = true });
    defer tmp_file.close();

    var writer_buffer: [4096]u8 = undefined;
    var file_writer = tmp_file.writer(&writer_buffer);
    try std.json.Stringify.value(root, .{ .whitespace = .indent_2 }, &file_writer.interface);
    try file_writer.interface.writeAll("\n");
    try file_writer.interface.flush();

    std.fs.cwd().rename(tmp_path, repos_json_path) catch |err| switch (err) {
        error.PathAlreadyExists => {
            try std.fs.cwd().deleteFile(repos_json_path);
            try std.fs.cwd().rename(tmp_path, repos_json_path);
        },
        else => return err,
    };
}

fn run(allocator: Allocator) !u8 {
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    if (args.len == 2 and (std.mem.eql(u8, args[1], "-h") or std.mem.eql(u8, args[1], "--help"))) {
        printUsage(args[0]);
        return 0;
    }

    if (args.len < 2 or args.len > 3) {
        printUsage(args[0]);
        return 1;
    }

    const src_raw = args[1];
    const src_abs = std.fs.cwd().realpathAlloc(allocator, src_raw) catch {
        std.debug.print("Error: source directory does not exist: {s}\n", .{src_raw});
        return 1;
    };

    var source_dir = std.fs.openDirAbsolute(src_abs, .{}) catch {
        std.debug.print("Error: source path is not a directory: {s}\n", .{src_raw});
        return 1;
    };
    source_dir.close();

    const dest_name: []const u8 = if (args.len >= 3 and args[2].len != 0)
        args[2]
    else
        std.fs.path.basename(src_abs);

    const repos_json_path = "repos.json";
    const file_content = std.fs.cwd().readFileAlloc(allocator, repos_json_path, 16 * 1024 * 1024) catch |err| switch (err) {
        error.FileNotFound => null,
        else => return err,
    };

    var parsed: ?std.json.Parsed(JsonValue) = null;
    defer {
        if (parsed) |*p| p.deinit();
    }

    var root: JsonValue = undefined;
    if (file_content) |content| {
        defer allocator.free(content);

        parsed = std.json.parseFromSlice(JsonValue, allocator, content, .{
            .allocate = .alloc_always,
        }) catch {
            std.debug.print("Error: repos.json is not valid JSON\n", .{});
            return 1;
        };
        root = parsed.?.value;
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

    const home_dir = try detectHomeDir(allocator);
    const cwd_abs = try std.fs.cwd().realpathAlloc(allocator, ".");
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
        std.debug.print("OK: repos.json updated: {s} -> {s}\n", .{ src_abs, dest_name });
    }

    return 0;
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

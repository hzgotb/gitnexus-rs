const builtin = @import("builtin");
const std = @import("std");
const container_group = @import("../container_group.zig");
const container_runtime = @import("../container_runtime.zig");
const i18n = @import("../i18n.zig");

const Allocator = std.mem.Allocator;
const JsonValue = std.json.Value;
const help_en = @embedFile("../i18n/analyze_help.en.txt");

const container_name_prefix = "gitnexus";
const registry_file_name = "registry.json";
const max_registry_file_size: usize = 16 * 1024 * 1024;

const Container = struct {
    id: []const u8,
    name: []const u8,
    status: []const u8,
};

const ActiveAnalyze = struct {
    repo_index: usize,
    child: std.process.Child,
};

const RegistryRepoState = struct {
    has_index: bool = false,
    indexed_at_raw: ?[]const u8 = null,
};

const RepoDisplayEntry = struct {
    name: []const u8,
    has_index: bool = false,
    indexed_at_raw: ?[]const u8 = null,
    indexed_at_display: ?[]const u8 = null,
};

const RepoState = enum {
    pending,
    running,
    succeeded,
    failed,
};

const OutputMode = enum {
    inherit,
    ignore,
    pipe,
};

fn warnInvalidRegistryFile(path: []const u8) void {
    std.debug.print(
        "Warning: registry file has unexpected JSON shape, ignoring: {s}\n",
        .{path},
    );
}

fn printUsage(allocator: Allocator, exe_name: []const u8) !void {
    try i18n.printHelpTemplate(allocator, help_en, exe_name);
}

fn resolvePathFromCwd(
    allocator: Allocator,
    cwd_abs: []const u8,
    raw_path: []const u8,
) ![]const u8 {
    if (std.fs.path.isAbsolute(raw_path)) {
        return try std.fs.path.resolve(allocator, &.{raw_path});
    }
    return try std.fs.path.resolve(allocator, &.{ cwd_abs, raw_path });
}

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn exitCodeFromTerm(term: std.process.Child.Term) u8 {
    return switch (term) {
        .Exited => |code| @as(u8, @intCast(if (code > 255) 1 else code)),
        else => 1,
    };
}

fn termFromWaitStatus(status: u32) std.process.Child.Term {
    return if (std.posix.W.IFEXITED(status))
        .{ .Exited = std.posix.W.EXITSTATUS(status) }
    else if (std.posix.W.IFSIGNALED(status))
        .{ .Signal = std.posix.W.TERMSIG(status) }
    else if (std.posix.W.IFSTOPPED(status))
        .{ .Stopped = std.posix.W.STOPSIG(status) }
    else
        .{ .Unknown = status };
}

fn countRunningStates(states: []const RepoState) usize {
    var count: usize = 0;
    for (states) |state| {
        if (state == .running) count += 1;
    }
    return count;
}

fn stateMarker(state: RepoState) []const u8 {
    return switch (state) {
        .pending => " ",
        .running => "~",
        .succeeded => "x",
        .failed => "!",
    };
}

fn fillProgressBar(buf: *[8]u8, state: RepoState, tick: usize) []const u8 {
    buf[0] = '[';
    buf[7] = ']';

    switch (state) {
        .pending => {
            @memset(buf[1..7], '-');
        },
        .succeeded => {
            @memset(buf[1..7], '#');
        },
        .failed => {
            @memset(buf[1..7], '!');
        },
        .running => {
            @memset(buf[1..7], '-');
            const width: usize = 6;
            const head = tick % width;
            var k: usize = 0;
            while (k < 2) : (k += 1) {
                const idx = (head + k) % width;
                buf[1 + idx] = '=';
            }
        },
    }

    return buf[0..];
}

fn formatIndexedAtUtcForDisplay(buf: []u8, indexed_at_raw: []const u8) []const u8 {
    const trimmed = std.mem.trim(u8, indexed_at_raw, " \t\r\n");
    if (trimmed.len < 20) return trimmed;
    if (trimmed[10] != 'T') return trimmed;

    var time_end = trimmed.len;
    if (trimmed[trimmed.len - 1] == 'Z') {
        time_end -= 1;
    }
    if (std.mem.indexOfScalarPos(u8, trimmed, 11, '.')) |dot| {
        if (dot < time_end) time_end = dot;
    }
    if (time_end <= 11) return trimmed;

    return std.fmt.bufPrint(buf, "{s} {s} UTC", .{
        trimmed[0..10],
        trimmed[11..time_end],
    }) catch trimmed;
}

fn parseFixedUnsigned(text: []const u8) !u32 {
    if (text.len == 0) return error.InvalidTimestamp;
    var value: u32 = 0;
    for (text) |ch| {
        if (!std.ascii.isDigit(ch)) return error.InvalidTimestamp;
        value = value * 10 + (ch - '0');
    }
    return value;
}

fn daysFromCivil(year: i64, month: i64, day: i64) i64 {
    const adjusted_year = year - @as(i64, if (month <= 2) 1 else 0);
    const era = @divFloor(adjusted_year, 400);
    const year_of_era = adjusted_year - era * 400;
    const shifted_month = month + @as(i64, if (month > 2) -3 else 9);
    const day_of_year = @divFloor(153 * shifted_month + 2, 5) + day - 1;
    const day_of_era = year_of_era * 365 + @divFloor(year_of_era, 4) - @divFloor(year_of_era, 100) + day_of_year;
    return era * 146097 + day_of_era - 719468;
}

fn parseIso8601UtcToUnixSeconds(indexed_at_raw: []const u8) !i64 {
    const trimmed = std.mem.trim(u8, indexed_at_raw, " \t\r\n");
    if (trimmed.len < 20) return error.InvalidTimestamp;
    if (trimmed[4] != '-' or trimmed[7] != '-' or trimmed[10] != 'T' or trimmed[13] != ':' or trimmed[16] != ':') {
        return error.InvalidTimestamp;
    }

    const year = try parseFixedUnsigned(trimmed[0..4]);
    const month = try parseFixedUnsigned(trimmed[5..7]);
    const day = try parseFixedUnsigned(trimmed[8..10]);
    const hour = try parseFixedUnsigned(trimmed[11..13]);
    const minute = try parseFixedUnsigned(trimmed[14..16]);
    const second = try parseFixedUnsigned(trimmed[17..19]);

    if (month < 1 or month > 12) return error.InvalidTimestamp;
    if (day < 1 or day > 31) return error.InvalidTimestamp;
    if (hour > 23 or minute > 59 or second > 60) return error.InvalidTimestamp;

    if (trimmed[19] == '.') {
        var idx: usize = 20;
        while (idx < trimmed.len and std.ascii.isDigit(trimmed[idx])) : (idx += 1) {}
        if (idx >= trimmed.len or trimmed[idx] != 'Z') return error.InvalidTimestamp;
    } else if (trimmed[19] != 'Z') {
        return error.InvalidTimestamp;
    }

    const days = daysFromCivil(year, month, day);
    return days * 24 * 60 * 60 +
        @as(i64, hour) * 60 * 60 +
        @as(i64, minute) * 60 +
        @as(i64, second);
}

fn formatUnixSecondsLocal(allocator: Allocator, unix_seconds: i64) !?[]const u8 {
    const format_arg = "+%Y-%m-%d %H:%M:%S %z";

    const result = switch (builtin.os.tag) {
        .linux => blk: {
            const at_arg = try std.fmt.allocPrint(allocator, "@{d}", .{unix_seconds});
            break :blk std.process.Child.run(.{
                .allocator = allocator,
                .argv = &.{ "date", "-d", at_arg, format_arg },
            }) catch return null;
        },
        .macos, .ios, .tvos, .watchos, .visionos, .freebsd, .openbsd, .netbsd, .dragonfly => blk: {
            const seconds_arg = try std.fmt.allocPrint(allocator, "{d}", .{unix_seconds});
            break :blk std.process.Child.run(.{
                .allocator = allocator,
                .argv = &.{ "date", "-r", seconds_arg, format_arg },
            }) catch return null;
        },
        else => return null,
    };

    if (!isExitedZero(result.term)) return null;
    return std.mem.trim(u8, result.stdout, " \t\r\n");
}

fn makeIndexedAtDisplay(allocator: Allocator, indexed_at_raw: []const u8) ![]const u8 {
    if (parseIso8601UtcToUnixSeconds(indexed_at_raw)) |unix_seconds| {
        if (try formatUnixSecondsLocal(allocator, unix_seconds)) |local_text| {
            return local_text;
        }
    } else |_| {}

    var fallback_buf: [40]u8 = undefined;
    return try allocator.dupe(u8, formatIndexedAtUtcForDisplay(&fallback_buf, indexed_at_raw));
}

fn repoSelectionHint() []const u8 {
    return "Use Up/Down arrows, Enter to confirm, q/c to cancel, Ctrl+C to exit";
}

fn repoHistoryGroupLabel() []const u8 {
    return "Indexed";
}

fn repoFreshGroupLabel() []const u8 {
    return "Not indexed";
}

fn repoLastOperatedLabel() []const u8 {
    return "Indexed at";
}

fn cancelledMessage() []const u8 {
    return "Cancelled.";
}

fn isCancelInput(input: []const u8) bool {
    if (input.len == 0) return false;
    if (std.ascii.eqlIgnoreCase(input, "q")) return true;
    if (std.ascii.eqlIgnoreCase(input, "quit")) return true;
    if (std.ascii.eqlIgnoreCase(input, "c")) return true;
    if (std.ascii.eqlIgnoreCase(input, "cancel")) return true;
    if (std.ascii.eqlIgnoreCase(input, "exit")) return true;
    return false;
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

fn extractRegistryRepoKey(obj: *const std.json.ObjectMap) []const u8 {
    const path = firstStringField(obj, &.{"path"});
    if (path.len != 0) {
        const base = std.fs.path.basename(path);
        if (base.len != 0) return base;
    }
    return firstStringField(obj, &.{"name"});
}

fn shouldReplaceRegistryState(existing: RegistryRepoState, candidate: RegistryRepoState) bool {
    if (existing.indexed_at_raw == null) return candidate.indexed_at_raw != null;
    if (candidate.indexed_at_raw == null) return false;
    return std.mem.order(u8, existing.indexed_at_raw.?, candidate.indexed_at_raw.?) == .lt;
}

fn repoDisplayLessThan(_: void, lhs: RepoDisplayEntry, rhs: RepoDisplayEntry) bool {
    if (lhs.has_index != rhs.has_index) return lhs.has_index;

    const lhs_has_time = lhs.indexed_at_raw != null;
    const rhs_has_time = rhs.indexed_at_raw != null;
    if (lhs_has_time != rhs_has_time) return lhs_has_time;
    if (lhs_has_time and rhs_has_time and !std.mem.eql(u8, lhs.indexed_at_raw.?, rhs.indexed_at_raw.?)) {
        return std.mem.order(u8, lhs.indexed_at_raw.?, rhs.indexed_at_raw.?) == .gt;
    }

    return std.mem.order(u8, lhs.name, rhs.name) == .lt;
}

fn buildRepoDisplayEntries(
    allocator: Allocator,
    repos: []const []const u8,
    registry_state: *const std.StringHashMap(RegistryRepoState),
) !std.ArrayList(RepoDisplayEntry) {
    var display_entries = std.ArrayList(RepoDisplayEntry).empty;
    for (repos) |repo| {
        const state = registry_state.get(repo) orelse RegistryRepoState{};
        try display_entries.append(allocator, .{
            .name = repo,
            .has_index = state.has_index,
            .indexed_at_raw = state.indexed_at_raw,
            .indexed_at_display = if (state.indexed_at_raw) |raw| try makeIndexedAtDisplay(allocator, raw) else null,
        });
    }
    std.mem.sort(RepoDisplayEntry, display_entries.items, {}, repoDisplayLessThan);
    return display_entries;
}

fn repoSelectionHeaderCount(repos: []const RepoDisplayEntry) usize {
    var has_history = false;
    var has_fresh = false;
    for (repos) |repo| {
        if (repo.has_index) {
            has_history = true;
        } else {
            has_fresh = true;
        }
    }
    return @as(usize, @intFromBool(has_history)) + @as(usize, @intFromBool(has_fresh));
}

fn printRepoSelectionHeader(clear_line: bool, label: []const u8) void {
    if (clear_line) {
        std.debug.print("\x1b[2K\r  {s}\n", .{label});
    } else {
        std.debug.print("  {s}\n", .{label});
    }
}

fn printRepoSelectionEntry(
    clear_line: bool,
    prefix: []const u8,
    display_index: usize,
    repo: RepoDisplayEntry,
) void {
    if (repo.indexed_at_display) |time_text| {
        const last_label = repoLastOperatedLabel();
        if (clear_line) {
            std.debug.print(
                "\x1b[2K\r{s}[{d}] {s} ({s}: {s})\n",
                .{ prefix, display_index, repo.name, last_label, time_text },
            );
        } else {
            std.debug.print(
                "{s}[{d}] {s} ({s}: {s})\n",
                .{ prefix, display_index, repo.name, last_label, time_text },
            );
        }
    } else if (clear_line) {
        std.debug.print("\x1b[2K\r{s}[{d}] {s}\n", .{ prefix, display_index, repo.name });
    } else {
        std.debug.print("{s}[{d}] {s}\n", .{ prefix, display_index, repo.name });
    }
}

fn printRepoSelectionList(
    repos: []const RepoDisplayEntry,
    selected: ?usize,
    clear_line: bool,
) void {
    var printed_history_header = false;
    var printed_fresh_header = false;

    for (repos, 0..) |repo, idx| {
        if (repo.has_index and !printed_history_header) {
            printed_history_header = true;
            printRepoSelectionHeader(clear_line, repoHistoryGroupLabel());
        } else if (!repo.has_index and !printed_fresh_header) {
            printed_fresh_header = true;
            printRepoSelectionHeader(clear_line, repoFreshGroupLabel());
        }

        const prefix = if (selected) |selected_idx|
            if (selected_idx == idx) "> " else "  "
        else
            "  ";
        printRepoSelectionEntry(clear_line, prefix, idx + 1, repo);
    }
}

fn cleanLineForPanelRaw(allocator: Allocator, line: []const u8) ![]const u8 {
    var stripped = std.ArrayList(u8).empty;
    var i: usize = 0;
    while (i < line.len) {
        const ch = line[i];
        if (ch == 0x1b) {
            if (i + 1 < line.len and line[i + 1] == '[') {
                i += 2;
                while (i < line.len) : (i += 1) {
                    const end = line[i];
                    if (end >= '@' and end <= '~') {
                        i += 1;
                        break;
                    }
                }
                continue;
            }
            i += 1;
            continue;
        }
        if (ch == '\t') {
            try stripped.append(allocator, ' ');
            i += 1;
            continue;
        }
        if (ch < 0x20) {
            i += 1;
            continue;
        }
        try stripped.append(allocator, ch);
        i += 1;
    }

    const trimmed = std.mem.trim(u8, stripped.items, " \t\r\n");
    return try allocator.dupe(u8, trimmed);
}

fn shortenLineTail(allocator: Allocator, line: []const u8, max_len: usize) ![]const u8 {
    if (line.len <= max_len) return try allocator.dupe(u8, line);
    if (max_len <= 3) return try allocator.dupe(u8, line[line.len - max_len ..]);
    return try std.fmt.allocPrint(allocator, "...{s}", .{line[line.len - (max_len - 3) ..]});
}

fn shortenLineHead(allocator: Allocator, line: []const u8, max_len: usize) ![]const u8 {
    if (line.len <= max_len) return try allocator.dupe(u8, line);
    if (max_len <= 3) return try allocator.dupe(u8, line[0..max_len]);
    return try std.fmt.allocPrint(allocator, "{s}...", .{line[0 .. max_len - 3]});
}

fn containsIgnoreCaseAscii(haystack: []const u8, needle: []const u8) bool {
    if (needle.len == 0) return true;
    if (haystack.len < needle.len) return false;
    var i: usize = 0;
    while (i + needle.len <= haystack.len) : (i += 1) {
        if (std.ascii.eqlIgnoreCase(haystack[i .. i + needle.len], needle)) return true;
    }
    return false;
}

fn hasAsciiDigit(text: []const u8) bool {
    for (text) |ch| {
        if (std.ascii.isDigit(ch)) return true;
    }
    return false;
}

fn captureCountSummaryLine(
    line: []const u8,
    full_summary: *[]const u8,
    nodes_summary: *[]const u8,
    edges_summary: *[]const u8,
) void {
    if (!hasAsciiDigit(line)) return;
    // Prefer the original final summary style:
    // "<nodes> nodes | <edges> edges | <clusters> clusters | <flows> flows"
    if (containsIgnoreCaseAscii(line, "nodes") and containsIgnoreCaseAscii(line, "edges")) {
        full_summary.* = line;
    }
    if (containsIgnoreCaseAscii(line, "nodes")) {
        nodes_summary.* = line;
    }
    if (containsIgnoreCaseAscii(line, "edges")) {
        edges_summary.* = line;
    }
}

fn composeSummaryMessage(
    allocator: Allocator,
    full_summary: []const u8,
    nodes_summary: []const u8,
    edges_summary: []const u8,
) !?[]const u8 {
    if (full_summary.len != 0) return full_summary;
    if (nodes_summary.len == 0 and edges_summary.len == 0) return null;
    if (nodes_summary.len != 0 and edges_summary.len != 0) {
        if (std.mem.eql(u8, nodes_summary, edges_summary)) return nodes_summary;
        return try std.fmt.allocPrint(allocator, "{s}; {s}", .{ nodes_summary, edges_summary });
    }
    if (nodes_summary.len != 0) return nodes_summary;
    return edges_summary;
}

fn updatePanelMessageFromPartial(
    allocator: Allocator,
    partial: *std.ArrayList(u8),
    current_message: *[]const u8,
    full_summary: *[]const u8,
    nodes_summary: *[]const u8,
    edges_summary: *[]const u8,
) !bool {
    if (partial.items.len == 0) return false;
    const cleaned_raw = try cleanLineForPanelRaw(allocator, partial.items);
    if (cleaned_raw.len == 0) return false;
    captureCountSummaryLine(cleaned_raw, full_summary, nodes_summary, edges_summary);

    const display_text = try shortenLineTail(allocator, cleaned_raw, 24);
    if (std.mem.eql(u8, display_text, current_message.*)) return false;
    current_message.* = display_text;
    return true;
}

fn consumeOutputChunk(
    allocator: Allocator,
    partial: *std.ArrayList(u8),
    current_message: *[]const u8,
    full_summary: *[]const u8,
    nodes_summary: *[]const u8,
    edges_summary: *[]const u8,
    chunk: []const u8,
) !bool {
    var changed = false;
    for (chunk) |ch| {
        if (ch == '\n' or ch == '\r') {
            if (try updatePanelMessageFromPartial(
                allocator,
                partial,
                current_message,
                full_summary,
                nodes_summary,
                edges_summary,
            )) {
                changed = true;
            }
            partial.clearRetainingCapacity();
            continue;
        }
        if (partial.items.len < 4096) {
            try partial.append(allocator, ch);
        }
    }
    return changed;
}

fn drainPipeToPanelMessage(
    allocator: Allocator,
    pipe_file: *?std.fs.File,
    partial: *std.ArrayList(u8),
    current_message: *[]const u8,
    full_summary: *[]const u8,
    nodes_summary: *[]const u8,
    edges_summary: *[]const u8,
) !bool {
    if (pipe_file.* == null) return false;
    var changed = false;
    var read_buf: [2048]u8 = undefined;
    while (true) {
        const n = pipe_file.*.?.read(&read_buf) catch |err| switch (err) {
            error.WouldBlock => break,
            else => return err,
        };
        if (n == 0) {
            pipe_file.*.?.close();
            pipe_file.* = null;
            break;
        }
        if (try consumeOutputChunk(
            allocator,
            partial,
            current_message,
            full_summary,
            nodes_summary,
            edges_summary,
            read_buf[0..n],
        )) {
            changed = true;
        }
    }
    return changed;
}

fn panelPrint(comptime fmt: []const u8, args: anytype) void {
    var buf: [1024]u8 = undefined;
    const rendered = std.fmt.bufPrint(&buf, fmt, args) catch return;
    std.fs.File.stdout().writeAll(rendered) catch {};
}

fn renderAnalyzePanel(
    repos: []const []const u8,
    states: []const RepoState,
    messages: []const []const u8,
    jobs: usize,
    completed_count: usize,
    failed_count: usize,
    tick: usize,
    rendered_once: *bool,
) void {
    const panel_lines = repos.len + 1;
    if (rendered_once.*) {
        // Move up by panel height and redraw in place.
        panelPrint("\x1b[{d}A\r", .{panel_lines});
    } else {
        rendered_once.* = true;
    }

    const running_count = countRunningStates(states);
    panelPrint(
        "\x1b[2K\rP {d}/{d} R{d} F{d} J{d}\n",
        .{ completed_count, repos.len, running_count, failed_count, jobs },
    );
    for (repos, 0..) |repo, idx| {
        var bar_buf: [8]u8 = undefined;
        const bar = fillProgressBar(&bar_buf, states[idx], tick + idx);
        const repo_max = 10;
        const repo_short = repo.len > repo_max;
        const repo_prefix = if (repo_short) repo[0 .. repo_max - 3] else repo;
        if (messages[idx].len == 0) {
            if (repo_short) {
                panelPrint("\x1b[2K\r[{s}] {s}... {s}\n", .{ stateMarker(states[idx]), repo_prefix, bar });
            } else {
                panelPrint("\x1b[2K\r[{s}] {s} {s}\n", .{ stateMarker(states[idx]), repo, bar });
            }
        } else {
            if (repo_short) {
                panelPrint(
                    "\x1b[2K\r[{s}] {s}... {s} | {s}\n",
                    .{ stateMarker(states[idx]), repo_prefix, bar, messages[idx] },
                );
            } else {
                panelPrint(
                    "\x1b[2K\r[{s}] {s} {s} | {s}\n",
                    .{ stateMarker(states[idx]), repo, bar, messages[idx] },
                );
            }
        }
    }
}

fn isManagedContainerName(name: []const u8) bool {
    return std.mem.eql(u8, name, container_name_prefix) or std.mem.startsWith(u8, name, container_name_prefix ++ "-");
}

fn readLineAlloc(allocator: Allocator) ![]u8 {
    var stdin_file = std.fs.File.stdin();
    var buf = std.ArrayList(u8).empty;

    while (true) {
        var byte: [1]u8 = undefined;
        const n = try stdin_file.read(&byte);
        if (n == 0) break;
        if (byte[0] == '\n') break;
        if (byte[0] == '\r') continue;
        try buf.append(allocator, byte[0]);
    }

    return try buf.toOwnedSlice(allocator);
}

fn promptLine(allocator: Allocator, prompt: []const u8) ![]const u8 {
    std.debug.print("{s}", .{prompt});
    return try readLineAlloc(allocator);
}

fn runCommandInherit(allocator: Allocator, argv: []const []const u8) !std.process.Child.Term {
    var child = std.process.Child.init(argv, allocator);
    child.stdin_behavior = .Inherit;
    child.stdout_behavior = .Inherit;
    child.stderr_behavior = .Inherit;
    try child.spawn();
    return try child.wait();
}

fn getSttyState(allocator: Allocator) ![]const u8 {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;

    var child = std.process.Child.init(&.{ "stty", "-g" }, allocator);
    child.stdin_behavior = .Inherit;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Ignore;
    try child.spawn();

    const stdout = try child.stdout.?.deprecatedReader().readAllAlloc(allocator, 1024);
    const term = try child.wait();
    if (!isExitedZero(term)) return error.SttyFailed;

    return std.mem.trim(u8, stdout, " \t\r\n");
}

fn setSttyRawNoEcho(allocator: Allocator) !void {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;
    const term = try runCommandInherit(allocator, &.{ "stty", "raw", "-echo" });
    if (!isExitedZero(term)) return error.SttyFailed;
}

fn restoreStty(allocator: Allocator, state: []const u8) !void {
    if (builtin.os.tag == .windows) return;
    var argv = [_][]const u8{ "stty", state };
    const term = try runCommandInherit(allocator, &argv);
    if (!isExitedZero(term)) return error.SttyFailed;
}

fn printCommandOutput(result: std.process.Child.RunResult) void {
    if (result.stdout.len != 0) {
        std.debug.print("{s}", .{result.stdout});
        if (result.stdout[result.stdout.len - 1] != '\n') {
            std.debug.print("\n", .{});
        }
    }
    if (result.stderr.len != 0) {
        std.debug.print("{s}", .{result.stderr});
        if (result.stderr[result.stderr.len - 1] != '\n') {
            std.debug.print("\n", .{});
        }
    }
}

fn runDocker(allocator: Allocator, argv: []const []const u8) !std.process.Child.RunResult {
    return container_runtime.runDocker(allocator, argv);
}

fn loadRegistryRepoState(allocator: Allocator, path: []const u8) !std.StringHashMap(RegistryRepoState) {
    var registry_state = std.StringHashMap(RegistryRepoState).init(allocator);

    const content = std.fs.cwd().readFileAlloc(allocator, path, max_registry_file_size) catch |err| switch (err) {
        error.FileNotFound => return registry_state,
        else => return err,
    };
    defer allocator.free(content);

    if (std.mem.trim(u8, content, " \t\r\n").len == 0) return registry_state;

    var parsed = std.json.parseFromSlice(JsonValue, allocator, content, .{
        .allocate = .alloc_always,
    }) catch {
        std.debug.print("Warning: registry file is not valid JSON, ignoring: {s}\n", .{path});
        return registry_state;
    };
    defer parsed.deinit();

    const root_array = switch (parsed.value) {
        .array => |arr| arr,
        else => {
            warnInvalidRegistryFile(path);
            return registry_state;
        },
    };

    for (root_array.items) |*entry| {
        const obj = switch (entry.*) {
            .object => |*obj| obj,
            else => continue,
        };
        const repo_key = extractRegistryRepoKey(obj);
        if (repo_key.len == 0) continue;

        const indexed_at = firstStringField(obj, &.{"indexedAt"});
        const candidate = RegistryRepoState{
            .has_index = true,
            .indexed_at_raw = if (indexed_at.len == 0) null else try allocator.dupe(u8, indexed_at),
        };

        if (registry_state.getPtr(repo_key)) |existing| {
            if (shouldReplaceRegistryState(existing.*, candidate)) {
                existing.* = candidate;
            }
            continue;
        }

        try registry_state.put(try allocator.dupe(u8, repo_key), candidate);
    }

    return registry_state;
}

fn collectRunningContainers(allocator: Allocator) !std.ArrayList(Container) {
    const result = try runDocker(allocator, &.{
        "docker",
        "ps",
        "--format",
        "{{.ID}}\t{{.Names}}\t{{.Status}}",
    });
    if (!isExitedZero(result.term)) {
        std.debug.print("Error: failed to list running containers.\n", .{});
        printCommandOutput(result);
        return error.DockerPsFailed;
    }

    var containers = std.ArrayList(Container).empty;
    var lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;

        var cols = std.mem.splitScalar(u8, line, '\t');
        const id = cols.next() orelse continue;
        const name = cols.next() orelse continue;
        const status = cols.next() orelse "";
        if (!isManagedContainerName(name)) continue;

        try containers.append(allocator, .{
            .id = try allocator.dupe(u8, id),
            .name = try allocator.dupe(u8, name),
            .status = try allocator.dupe(u8, status),
        });
    }

    return containers;
}

fn collectReposInContainer(allocator: Allocator, container_name: []const u8) !std.ArrayList([]const u8) {
    const result = try runDocker(allocator, &.{
        "docker",
        "exec",
        container_name,
        "sh",
        "-lc",
        "for p in /repos/*; do [ -d \"$p\" ] && basename \"$p\"; done",
    });
    if (!isExitedZero(result.term)) {
        std.debug.print("Error: failed to list repos in container: {s}\n", .{container_name});
        printCommandOutput(result);
        return error.ListReposFailed;
    }

    var repos = std.ArrayList([]const u8).empty;
    var lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;
        try repos.append(allocator, try allocator.dupe(u8, line));
    }

    return repos;
}

fn selectContainerByPrompt(allocator: Allocator, containers: []const Container) !usize {
    for (containers, 0..) |container, idx| {
        std.debug.print("  [{d}] {s} ({s})\n", .{ idx + 1, container.name, container.status });
    }
    const prompt = try std.fmt.allocPrint(
        allocator,
        "Select container (1-{d}, default 1, q/c cancel): ",
        .{containers.len},
    );
    const input = try promptLine(allocator, prompt);
    const trimmed = std.mem.trim(u8, input, " \t\r\n");

    if (trimmed.len == 0) return 0;
    if (isCancelInput(trimmed)) return error.UserCancelled;
    const parsed = std.fmt.parseInt(usize, trimmed, 10) catch return 0;
    if (parsed < 1 or parsed > containers.len) return 0;
    return parsed - 1;
}

fn findContainerBySelector(containers: []const Container, selector: []const u8) !usize {
    var exact_idx: ?usize = null;
    for (containers, 0..) |container, idx| {
        if (std.mem.eql(u8, container.name, selector) or std.mem.eql(u8, container.id, selector)) {
            if (exact_idx != null) return error.AmbiguousSelector;
            exact_idx = idx;
        }
    }
    if (exact_idx) |idx| return idx;

    var prefix_idx: ?usize = null;
    for (containers, 0..) |container, idx| {
        if (std.mem.startsWith(u8, container.id, selector)) {
            if (prefix_idx != null) return error.AmbiguousSelector;
            prefix_idx = idx;
        }
    }
    if (prefix_idx) |idx| return idx;

    return error.ContainerNotFound;
}

fn findRepoByName(repos: []const []const u8, repo_name: []const u8) !usize {
    var found_idx: ?usize = null;
    for (repos, 0..) |repo, idx| {
        if (std.mem.eql(u8, repo, repo_name)) {
            if (found_idx != null) return error.AmbiguousRepo;
            found_idx = idx;
        }
    }
    return found_idx orelse error.RepoNotFound;
}

fn selectRepoByPrompt(allocator: Allocator, repos: []const RepoDisplayEntry) !usize {
    printRepoSelectionList(repos, null, false);

    const prompt = try std.fmt.allocPrint(
        allocator,
        "Select repo (1-{d}, default 1, q/c cancel): ",
        .{repos.len},
    );
    const input = try promptLine(allocator, prompt);
    const trimmed = std.mem.trim(u8, input, " \t\r\n");

    if (trimmed.len == 0) return 0;
    if (isCancelInput(trimmed)) return error.UserCancelled;
    const parsed = std.fmt.parseInt(usize, trimmed, 10) catch return 0;
    if (parsed < 1 or parsed > repos.len) return 0;
    return parsed - 1;
}

fn selectRepoByArrow(allocator: Allocator, repos: []const RepoDisplayEntry) !usize {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;
    if (!std.fs.File.stdin().isTty()) return error.NotATerminal;
    if (repos.len == 0) return error.EmptyRepoList;

    const saved_stty = try getSttyState(allocator);
    try setSttyRawNoEcho(allocator);
    std.debug.print("\x1b[?25l", .{});
    defer {
        std.debug.print("\x1b[?25h", .{});
        restoreStty(allocator, saved_stty) catch {};
        std.debug.print("\r\n", .{});
    }

    var selected: usize = 0;
    var rendered_once = false;
    var stdin_file = std.fs.File.stdin();
    const rendered_lines = repos.len + repoSelectionHeaderCount(repos) + 1;

    while (true) {
        if (rendered_once) {
            std.debug.print("\x1b[{d}A", .{rendered_lines});
        } else {
            rendered_once = true;
        }

        std.debug.print("\x1b[2K\r{s}\n", .{repoSelectionHint()});
        printRepoSelectionList(repos, selected, true);

        var key: [1]u8 = undefined;
        const n = try stdin_file.read(&key);
        if (n == 0) break;

        switch (key[0]) {
            3 => return error.UserCancelled,
            '\n', '\r' => break,
            'q', 'Q', 'c', 'C' => return error.UserCancelled,
            'k', 'K' => {
                if (selected > 0) selected -= 1;
            },
            'j', 'J' => {
                if (selected + 1 < repos.len) selected += 1;
            },
            27 => {
                var seq: [2]u8 = undefined;
                const n1 = try stdin_file.read(seq[0..1]);
                if (n1 == 0) continue;
                const n2 = try stdin_file.read(seq[1..2]);
                if (n2 == 0) continue;

                if (seq[0] == '[' and seq[1] == 'A') {
                    if (selected > 0) selected -= 1;
                } else if (seq[0] == '[' and seq[1] == 'B') {
                    if (selected + 1 < repos.len) selected += 1;
                }
            },
            else => {},
        }
    }

    return selected;
}

fn selectRepoIndex(allocator: Allocator, repos: []const RepoDisplayEntry) !usize {
    return selectRepoByArrow(allocator, repos) catch {
        return try selectRepoByPrompt(allocator, repos);
    };
}

fn spawnAnalyzeInRepo(
    allocator: Allocator,
    container_name: []const u8,
    repo_name: []const u8,
    analyze_args: []const []const u8,
    attach_tty: bool,
    inherit_stdin: bool,
    output_mode: OutputMode,
) !std.process.Child {
    const workdir = try std.fmt.allocPrint(allocator, "/repos/{s}", .{repo_name});

    var docker_args = std.ArrayList([]const u8).empty;
    try docker_args.appendSlice(allocator, &.{ "docker", "exec" });

    if (attach_tty) {
        try docker_args.append(allocator, "-t");
    }
    if (inherit_stdin) {
        try docker_args.append(allocator, "-i");
    }

    try docker_args.appendSlice(allocator, &.{ "-w", workdir, container_name, "gitnexus", "analyze" });
    try docker_args.appendSlice(allocator, analyze_args);

    var child = try container_runtime.initDockerChild(allocator, docker_args.items);
    child.stdin_behavior = if (inherit_stdin) .Inherit else .Ignore;
    child.stdout_behavior = switch (output_mode) {
        .inherit => .Inherit,
        .ignore => .Ignore,
        .pipe => .Pipe,
    };
    child.stderr_behavior = switch (output_mode) {
        .inherit => .Inherit,
        .ignore => .Ignore,
        .pipe => .Pipe,
    };
    try child.spawn();
    try child.waitForSpawn();
    return child;
}

fn runAnalyzeInRepo(
    allocator: Allocator,
    container_name: []const u8,
    repo_name: []const u8,
    analyze_args: []const []const u8,
) !u8 {
    const use_tty = std.fs.File.stdin().isTty() and std.fs.File.stdout().isTty();
    var child = try spawnAnalyzeInRepo(
        allocator,
        container_name,
        repo_name,
        analyze_args,
        use_tty,
        true,
        .inherit,
    );
    return exitCodeFromTerm(try child.wait());
}

fn runAnalyzeAllRepos(
    allocator: Allocator,
    container_name: []const u8,
    repos: []const []const u8,
    analyze_args: []const []const u8,
    jobs: usize,
) !u8 {
    const stdout_is_tty = std.fs.File.stdout().isTty();
    const use_panel = stdout_is_tty and builtin.os.tag != .windows and builtin.os.tag != .wasi;
    std.debug.print("Running analyze for all repos in container: {s} (jobs={d})\n", .{
        container_name,
        jobs,
    });

    var states = try allocator.alloc(RepoState, repos.len);
    for (states) |*state| {
        state.* = .pending;
    }
    var messages = try allocator.alloc([]const u8, repos.len);
    for (messages) |*msg| {
        msg.* = "";
    }
    var node_summaries = try allocator.alloc([]const u8, repos.len);
    for (node_summaries) |*summary| {
        summary.* = "";
    }
    var full_summaries = try allocator.alloc([]const u8, repos.len);
    for (full_summaries) |*summary| {
        summary.* = "";
    }
    var edge_summaries = try allocator.alloc([]const u8, repos.len);
    for (edge_summaries) |*summary| {
        summary.* = "";
    }
    var stdout_partials = try allocator.alloc(std.ArrayList(u8), repos.len);
    for (stdout_partials) |*partial| {
        partial.* = std.ArrayList(u8).empty;
    }
    var stderr_partials = try allocator.alloc(std.ArrayList(u8), repos.len);
    for (stderr_partials) |*partial| {
        partial.* = std.ArrayList(u8).empty;
    }

    var active = std.ArrayList(ActiveAnalyze).empty;
    const PollSource = struct {
        active_index: usize,
        is_stderr: bool,
    };
    var pollfds = std.ArrayList(std.posix.pollfd).empty;
    var poll_sources = std.ArrayList(PollSource).empty;

    var next_repo_index: usize = 0;
    var completed_count: usize = 0;
    var failed_count: usize = 0;
    var render_tick: usize = 0;
    var panel_rendered = false;

    if (use_panel) {
        panelPrint("\x1b[?25l", .{});
        defer {
            panelPrint("\x1b[?25h", .{});
            if (panel_rendered) {
                panelPrint("\n", .{});
            }
        }
    } else {
        std.debug.print("[progress] 0/{d} completed, running 0, failed 0\n", .{repos.len});
    }

    while (next_repo_index < repos.len or active.items.len != 0) {
        var state_changed = false;

        while (next_repo_index < repos.len and active.items.len < jobs) {
            const repo_index = next_repo_index;
            const repo = repos[repo_index];
            states[repo_index] = .running;
            messages[repo_index] = "starting...";
            state_changed = true;

            if (!use_panel) {
                std.debug.print("\n[{s}] starting gitnexus analyze ({d}/{d})\n", .{
                    repo,
                    repo_index + 1,
                    repos.len,
                });
            }

            const child = spawnAnalyzeInRepo(
                allocator,
                container_name,
                repo,
                analyze_args,
                stdout_is_tty,
                false,
                if (use_panel) .pipe else .inherit,
            ) catch |err| {
                states[repo_index] = .failed;
                completed_count += 1;
                failed_count += 1;
                messages[repo_index] = try std.fmt.allocPrint(allocator, "start failed: {s}", .{@errorName(err)});
                state_changed = true;
                if (!use_panel) {
                    std.debug.print("[{s}] failed to start: {s}\n", .{ repo, @errorName(err) });
                }
                next_repo_index += 1;
                continue;
            };
            try active.append(allocator, .{
                .repo_index = repo_index,
                .child = child,
            });
            next_repo_index += 1;
        }

        var completed_any = false;

        if (builtin.os.tag == .windows or builtin.os.tag == .wasi) {
            if (active.items.len != 0) {
                var done = active.orderedRemove(0);
                const code = exitCodeFromTerm(try done.child.wait());
                const repo = repos[done.repo_index];
                states[done.repo_index] = if (code == 0) .succeeded else .failed;
                completed_count += 1;
                if (code != 0) failed_count += 1;
                if (messages[done.repo_index].len == 0 or std.mem.eql(u8, messages[done.repo_index], "starting...")) {
                    messages[done.repo_index] = if (code == 0) "done" else try std.fmt.allocPrint(
                        allocator,
                        "failed (exit {d})",
                        .{code},
                    );
                }
                state_changed = true;
                completed_any = true;
                if (!use_panel) {
                    if (code == 0) {
                        std.debug.print("[{s}] done\n", .{repo});
                    } else {
                        std.debug.print("[{s}] failed with exit code {d}\n", .{ repo, code });
                    }
                }
            }
        } else {
            if (use_panel and active.items.len != 0) {
                pollfds.clearRetainingCapacity();
                poll_sources.clearRetainingCapacity();
                const poll_events = std.posix.POLL.IN | std.posix.POLL.HUP | std.posix.POLL.ERR;

                for (active.items, 0..) |entry, active_index| {
                    if (entry.child.stdout) |stdout_file| {
                        try pollfds.append(allocator, .{
                            .fd = stdout_file.handle,
                            .events = poll_events,
                            .revents = 0,
                        });
                        try poll_sources.append(allocator, .{
                            .active_index = active_index,
                            .is_stderr = false,
                        });
                    }
                    if (entry.child.stderr) |stderr_file| {
                        try pollfds.append(allocator, .{
                            .fd = stderr_file.handle,
                            .events = poll_events,
                            .revents = 0,
                        });
                        try poll_sources.append(allocator, .{
                            .active_index = active_index,
                            .is_stderr = true,
                        });
                    }
                }

                if (pollfds.items.len != 0) {
                    _ = try std.posix.poll(pollfds.items, 120);
                    for (pollfds.items, 0..) |pfd, poll_idx| {
                        if (pfd.revents == 0) continue;
                        const source = poll_sources.items[poll_idx];
                        if (source.active_index >= active.items.len) continue;

                        var active_entry = &active.items[source.active_index];
                        if (source.is_stderr) {
                            if (active_entry.child.stderr == null) continue;
                            var read_buf: [2048]u8 = undefined;
                            const n = active_entry.child.stderr.?.read(&read_buf) catch |err| switch (err) {
                                error.WouldBlock => 0,
                                else => return err,
                            };
                            if (n == 0) {
                                active_entry.child.stderr.?.close();
                                active_entry.child.stderr = null;
                                continue;
                            }
                            const repo_idx = active_entry.repo_index;
                            if (try consumeOutputChunk(
                                allocator,
                                &stderr_partials[repo_idx],
                                &messages[repo_idx],
                                &full_summaries[repo_idx],
                                &node_summaries[repo_idx],
                                &edge_summaries[repo_idx],
                                read_buf[0..n],
                            )) {
                                state_changed = true;
                            }
                        } else {
                            if (active_entry.child.stdout == null) continue;
                            var read_buf: [2048]u8 = undefined;
                            const n = active_entry.child.stdout.?.read(&read_buf) catch |err| switch (err) {
                                error.WouldBlock => 0,
                                else => return err,
                            };
                            if (n == 0) {
                                active_entry.child.stdout.?.close();
                                active_entry.child.stdout = null;
                                continue;
                            }
                            const repo_idx = active_entry.repo_index;
                            if (try consumeOutputChunk(
                                allocator,
                                &stdout_partials[repo_idx],
                                &messages[repo_idx],
                                &full_summaries[repo_idx],
                                &node_summaries[repo_idx],
                                &edge_summaries[repo_idx],
                                read_buf[0..n],
                            )) {
                                state_changed = true;
                            }
                        }
                    }
                } else {
                    std.Thread.sleep(120 * std.time.ns_per_ms);
                }
            }

            var idx: usize = 0;
            while (idx < active.items.len) {
                const wait_result = std.posix.waitpid(active.items[idx].child.id, std.posix.W.NOHANG);
                if (wait_result.pid == 0) {
                    idx += 1;
                    continue;
                }

                var done = active.orderedRemove(idx);
                const term = termFromWaitStatus(wait_result.status);
                done.child.term = term;

                if (use_panel) {
                    if (try drainPipeToPanelMessage(
                        allocator,
                        &done.child.stdout,
                        &stdout_partials[done.repo_index],
                        &messages[done.repo_index],
                        &full_summaries[done.repo_index],
                        &node_summaries[done.repo_index],
                        &edge_summaries[done.repo_index],
                    )) {
                        state_changed = true;
                    }
                    if (try drainPipeToPanelMessage(
                        allocator,
                        &done.child.stderr,
                        &stderr_partials[done.repo_index],
                        &messages[done.repo_index],
                        &full_summaries[done.repo_index],
                        &node_summaries[done.repo_index],
                        &edge_summaries[done.repo_index],
                    )) {
                        state_changed = true;
                    }
                }

                _ = try done.child.wait();
                const code = exitCodeFromTerm(term);
                const repo = repos[done.repo_index];

                if (try updatePanelMessageFromPartial(
                    allocator,
                    &stdout_partials[done.repo_index],
                    &messages[done.repo_index],
                    &full_summaries[done.repo_index],
                    &node_summaries[done.repo_index],
                    &edge_summaries[done.repo_index],
                )) {
                    state_changed = true;
                }
                if (try updatePanelMessageFromPartial(
                    allocator,
                    &stderr_partials[done.repo_index],
                    &messages[done.repo_index],
                    &full_summaries[done.repo_index],
                    &node_summaries[done.repo_index],
                    &edge_summaries[done.repo_index],
                )) {
                    state_changed = true;
                }
                stdout_partials[done.repo_index].clearRetainingCapacity();
                stderr_partials[done.repo_index].clearRetainingCapacity();

                if (try composeSummaryMessage(
                    allocator,
                    full_summaries[done.repo_index],
                    node_summaries[done.repo_index],
                    edge_summaries[done.repo_index],
                )) |summary_message| {
                    const summary_display = try shortenLineHead(allocator, summary_message, 64);
                    if (!std.mem.eql(u8, summary_display, messages[done.repo_index])) {
                        messages[done.repo_index] = summary_display;
                        state_changed = true;
                    }
                }

                states[done.repo_index] = if (code == 0) .succeeded else .failed;
                completed_count += 1;
                if (code != 0) failed_count += 1;
                if (messages[done.repo_index].len == 0 or std.mem.eql(u8, messages[done.repo_index], "starting...")) {
                    messages[done.repo_index] = if (code == 0) "done" else try std.fmt.allocPrint(
                        allocator,
                        "failed (exit {d})",
                        .{code},
                    );
                }
                state_changed = true;
                completed_any = true;
                if (!use_panel) {
                    if (code == 0) {
                        std.debug.print("[{s}] done\n", .{repo});
                    } else {
                        std.debug.print("[{s}] failed with exit code {d}\n", .{ repo, code });
                    }
                }
            }
        }

        if (use_panel) {
            const has_running = countRunningStates(states) != 0;
            if (state_changed or has_running) {
                render_tick +%= 1;
                renderAnalyzePanel(
                    repos,
                    states,
                    messages,
                    jobs,
                    completed_count,
                    failed_count,
                    render_tick,
                    &panel_rendered,
                );
            }
        } else if (state_changed) {
            std.debug.print(
                "[progress] {d}/{d} completed, running {d}, failed {d}\n",
                .{ completed_count, repos.len, active.items.len, failed_count },
            );
        }

        if (!use_panel and !completed_any and (next_repo_index < repos.len or active.items.len != 0)) {
            std.Thread.sleep(120 * std.time.ns_per_ms);
        }
    }

    if (failed_count != 0) {
        std.debug.print(
            "\nError: analyze failed in {d}/{d} repos.\n",
            .{ failed_count, repos.len },
        );
        return 1;
    }

    std.debug.print("\nAnalyze succeeded for all {d} repos.\n", .{repos.len});
    return 0;
}

pub fn runWithArgs(allocator: Allocator, args: []const []const u8) !u8 {
    var container_selector: ?[]const u8 = null;
    var repo_selector: ?[]const u8 = null;
    var registry_override: ?[]const u8 = null;
    var analyze_all = false;
    var jobs: usize = 1;
    var passthrough_start: usize = args.len;

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "-h") or std.mem.eql(u8, arg, "--help")) {
            try printUsage(allocator, args[0]);
            return 0;
        }
        if (std.mem.eql(u8, arg, "-c") or std.mem.eql(u8, arg, "--container")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                try printUsage(allocator, args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: container selector must not be empty\n", .{});
                return 1;
            }
            container_selector = value;
            continue;
        }
        if (std.mem.eql(u8, arg, "-r") or std.mem.eql(u8, arg, "--repo")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                try printUsage(allocator, args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: repo selector must not be empty\n", .{});
                return 1;
            }
            repo_selector = value;
            continue;
        }
        if (std.mem.eql(u8, arg, "--registry")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                try printUsage(allocator, args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: registry path must not be empty\n", .{});
                return 1;
            }
            registry_override = value;
            continue;
        }
        if (std.mem.eql(u8, arg, "-a") or std.mem.eql(u8, arg, "--all")) {
            analyze_all = true;
            continue;
        }
        if (std.mem.eql(u8, arg, "-j") or std.mem.eql(u8, arg, "--jobs")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                try printUsage(allocator, args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: jobs must not be empty\n", .{});
                return 1;
            }
            const parsed = std.fmt.parseInt(usize, value, 10) catch {
                std.debug.print("Error: invalid jobs value: {s}\n", .{value});
                return 1;
            };
            if (parsed < 1) {
                std.debug.print("Error: jobs must be >= 1\n", .{});
                return 1;
            }
            jobs = parsed;
            continue;
        }
        if (std.mem.eql(u8, arg, "--")) {
            passthrough_start = i + 1;
            break;
        }

        passthrough_start = i;
        break;
    }

    const analyze_args = args[passthrough_start..];

    if (analyze_all and repo_selector != null) {
        std.debug.print("Error: --repo and --all cannot be used together.\n", .{});
        try printUsage(allocator, args[0]);
        return 1;
    }
    if (!analyze_all and jobs != 1) {
        std.debug.print("Error: --jobs requires --all.\n", .{});
        try printUsage(allocator, args[0]);
        return 1;
    }

    const containers = collectRunningContainers(allocator) catch |err| switch (err) {
        error.DockerPsFailed => return 1,
        else => return err,
    };
    if (containers.items.len == 0) {
        std.debug.print("Error: no running managed container found (expected names like {s} or {s}-*)\n", .{
            container_name_prefix,
            container_name_prefix,
        });
        return 1;
    }

    const target_index = if (container_selector) |selector|
        findContainerBySelector(containers.items, selector) catch |err| switch (err) {
            error.ContainerNotFound => {
                std.debug.print("Error: target container not found: {s}\n", .{selector});
                std.debug.print("Available running containers:\n", .{});
                for (containers.items) |container| {
                    std.debug.print("  {s} ({s})\n", .{ container.name, container.id });
                }
                return 1;
            },
            error.AmbiguousSelector => {
                std.debug.print("Error: ambiguous container selector: {s}\n", .{selector});
                std.debug.print("Please specify full name/id. Available:\n", .{});
                for (containers.items) |container| {
                    std.debug.print("  {s} ({s})\n", .{ container.name, container.id });
                }
                return 1;
            },
            else => return err,
        }
    else if (containers.items.len == 1)
        0
    else blk: {
        std.debug.print("Multiple running containers found:\n", .{});
        break :blk selectContainerByPrompt(allocator, containers.items) catch |err| switch (err) {
            error.UserCancelled => {
                std.debug.print("{s}\n", .{cancelledMessage()});
                return 130;
            },
            else => return err,
        };
    };

    const target = containers.items[target_index];
    std.debug.print("Using container: {s}\n", .{target.name});

    const cwd_abs = try std.fs.cwd().realpathAlloc(allocator, ".");
    const labeled_group_paths = container_group.inspectContainerJsonPaths(allocator, target.name) catch container_group.JsonGroupPaths{};
    const registry_path = if (registry_override) |path|
        try resolvePathFromCwd(allocator, cwd_abs, path)
    else if (labeled_group_paths.registry_json_path.len != 0)
        labeled_group_paths.registry_json_path
    else
        try std.fs.path.join(allocator, &.{ cwd_abs, registry_file_name });

    const repos = collectReposInContainer(allocator, target.name) catch |err| switch (err) {
        error.ListReposFailed => return 1,
        else => return err,
    };
    if (repos.items.len == 0) {
        std.debug.print("Error: no repo found under /repos in container: {s}\n", .{target.name});
        return 1;
    }

    if (analyze_all) return try runAnalyzeAllRepos(
        allocator,
        target.name,
        repos.items,
        analyze_args,
        jobs,
    );

    const target_repo = if (repo_selector) |repo_name|
        repos.items[
            findRepoByName(repos.items, repo_name) catch |err| switch (err) {
                error.RepoNotFound => {
                    std.debug.print("Error: repo not found in container: {s}\n", .{repo_name});
                    std.debug.print("Available repos:\n", .{});
                    for (repos.items) |repo| {
                        std.debug.print("  {s}\n", .{repo});
                    }
                    return 1;
                },
                else => return err,
            }
        ]
    else if (repos.items.len == 1)
        repos.items[0]
    else blk: {
        var registry_state = try loadRegistryRepoState(allocator, registry_path);
        const display_entries = try buildRepoDisplayEntries(allocator, repos.items, &registry_state);
        std.debug.print("Multiple repos found in container:\n", .{});
        const selected_index = selectRepoIndex(allocator, display_entries.items) catch |err| switch (err) {
            error.UserCancelled => {
                std.debug.print("{s}\n", .{cancelledMessage()});
                return 130;
            },
            else => return err,
        };
        break :blk display_entries.items[selected_index].name;
    };

    std.debug.print("Using repo: {s}\n", .{target_repo});
    return try runAnalyzeInRepo(allocator, target.name, target_repo, analyze_args);
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

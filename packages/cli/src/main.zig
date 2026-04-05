const std = @import("std");
const i18n = @import("i18n.zig");
const add_repo_cmd = @import("commands/add_repo.zig");
const analyze_cmd = @import("commands/analyze.zig");
const completion_cmd = @import("commands/completion.zig");
const doctor_cmd = @import("commands/doctor.zig");
const start_cmd = @import("commands/start.zig");
const main_help_en = @embedFile("i18n/main_help.en.txt");

const Allocator = std.mem.Allocator;

const BuiltinCommand = enum {
    help,
    doctor,
    completion,
    start,
    analyze,
    add_repo,
    external,
};

fn printUsage(allocator: Allocator, exe_name: []const u8) !void {
    try i18n.printHelpTemplate(allocator, main_help_en, exe_name);
}

fn exitCodeFromTerm(term: std.process.Child.Term) u8 {
    return switch (term) {
        .Exited => |code| @as(u8, @intCast(if (code > 255) 1 else code)),
        else => 1,
    };
}

fn classifyCommand(subcommand: []const u8) BuiltinCommand {
    if (std.mem.eql(u8, subcommand, "-h") or std.mem.eql(u8, subcommand, "--help") or std.mem.eql(u8, subcommand, "help")) {
        return .help;
    }
    if (std.mem.eql(u8, subcommand, "doctor") or std.mem.eql(u8, subcommand, "diagnose")) return .doctor;
    if (std.mem.eql(u8, subcommand, "completion") or std.mem.eql(u8, subcommand, "complete")) return .completion;
    if (std.mem.eql(u8, subcommand, "start")) return .start;
    if (std.mem.eql(u8, subcommand, "analyze")) return .analyze;
    if (std.mem.eql(u8, subcommand, "add-repo") or std.mem.eql(u8, subcommand, "add_repo")) return .add_repo;
    return .external;
}

const GlobalParseResult = struct {
    command_index: usize,
};

fn parseGlobalOptions(allocator: Allocator, args: []const []const u8) !GlobalParseResult {
    _ = allocator;
    _ = args;

    return .{
        .command_index = 1,
    };
}

fn runWithProgram(
    allocator: Allocator,
    program: []const u8,
    command_args: []const []const u8,
) !u8 {
    var argv = std.ArrayList([]const u8).empty;
    try argv.append(allocator, program);
    try argv.appendSlice(allocator, command_args);

    var child = std.process.Child.init(argv.items, allocator);
    child.stdin_behavior = .Inherit;
    child.stdout_behavior = .Inherit;
    child.stderr_behavior = .Inherit;
    try child.spawn();
    return exitCodeFromTerm(try child.wait());
}

fn runBuiltinCommand(
    allocator: Allocator,
    selected: BuiltinCommand,
    command_name: []const u8,
    command_args: []const []const u8,
) !u8 {
    const forward_prog_name = try std.fmt.allocPrint(allocator, "gitn {s}", .{command_name});
    var forwarded = try allocator.alloc([]const u8, command_args.len + 1);
    forwarded[0] = forward_prog_name;
    if (command_args.len != 0) {
        @memcpy(forwarded[1..], command_args);
    }

    return switch (selected) {
        .doctor => try doctor_cmd.runWithArgs(allocator, forwarded),
        .completion => try completion_cmd.runWithArgs(allocator, forwarded),
        .start => try start_cmd.runWithArgs(allocator, forwarded),
        .analyze => try analyze_cmd.runWithArgs(allocator, forwarded),
        .add_repo => try add_repo_cmd.runWithArgs(allocator, forwarded),
        else => unreachable,
    };
}

fn run(allocator: Allocator) !u8 {
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    const global = parseGlobalOptions(allocator, args) catch {
        try printUsage(allocator, args[0]);
        return 1;
    };

    if (global.command_index >= args.len) {
        try printUsage(allocator, args[0]);
        return 1;
    }

    const first = args[global.command_index];
    const rest = if (global.command_index + 1 < args.len) args[global.command_index + 1 ..] else &.{};
    const selected = classifyCommand(first);
    if (selected == .help) {
        try printUsage(allocator, args[0]);
        return 0;
    }
    if (selected != .external) {
        return runBuiltinCommand(allocator, selected, first, rest);
    }

    return runWithProgram(allocator, first, rest) catch |err| switch (err) {
        error.FileNotFound => {
            std.debug.print("Error: command not found: {s}\n", .{first});
            return 1;
        },
        else => return err,
    };
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

pub const testing = struct {
    pub const add_repo_module = add_repo_cmd;
    pub const completion_module = completion_cmd;

    pub fn globalCommandIndex(allocator: Allocator, args: []const []const u8) !usize {
        return (try parseGlobalOptions(allocator, args)).command_index;
    }

    pub fn commandIsExternal(subcommand: []const u8) bool {
        return classifyCommand(subcommand) == .external;
    }
};

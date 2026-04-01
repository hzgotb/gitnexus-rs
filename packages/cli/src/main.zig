const std = @import("std");
const i18n = @import("i18n.zig");
const add_repo_cmd = @import("commands/add_repo.zig");
const analyze_cmd = @import("commands/analyze.zig");
const completion_cmd = @import("commands/completion.zig");
const doctor_cmd = @import("commands/doctor.zig");
const start_cmd = @import("commands/start.zig");
const main_help_en = @embedFile("i18n/main_help.en.txt");
const main_help_zh = @embedFile("i18n/main_help.zh.txt");

const Allocator = std.mem.Allocator;
const UiLang = i18n.UiLang;

const BuiltinCommand = enum {
    help,
    doctor,
    completion,
    start,
    analyze,
    add_repo,
    external,
};

fn printUsage(allocator: Allocator, exe_name: []const u8, lang: UiLang) !void {
    const tpl = switch (lang) {
        .zh => main_help_zh,
        .en => main_help_en,
    };
    try i18n.printHelpTemplate(allocator, tpl, exe_name);
}

fn exitCodeFromTerm(term: std.process.Child.Term) u8 {
    return switch (term) {
        .Exited => |code| @as(u8, @intCast(if (code > 255) 1 else code)),
        else => 1,
    };
}

fn classifyCommand(subcommand: []const u8) BuiltinCommand {
    if (std.mem.eql(u8, subcommand, "-h") or std.mem.eql(u8, subcommand, "--help") or std.mem.eql(u8, subcommand, "help") or std.mem.eql(u8, subcommand, "帮助")) {
        return .help;
    }
    if (std.mem.eql(u8, subcommand, "doctor") or std.mem.eql(u8, subcommand, "diagnose") or std.mem.eql(u8, subcommand, "诊断")) return .doctor;
    if (std.mem.eql(u8, subcommand, "completion") or std.mem.eql(u8, subcommand, "complete")) return .completion;
    if (std.mem.eql(u8, subcommand, "start")) return .start;
    if (std.mem.eql(u8, subcommand, "analyze")) return .analyze;
    if (std.mem.eql(u8, subcommand, "add-repo") or std.mem.eql(u8, subcommand, "add_repo")) return .add_repo;
    return .external;
}

const GlobalParseResult = struct {
    lang: UiLang,
    command_index: usize,
};

fn parseGlobalOptions(allocator: Allocator, args: []const []const u8) !GlobalParseResult {
    var lang = i18n.detectLangFromEnv(allocator);
    var idx: usize = 1;

    while (idx < args.len) {
        const arg = args[idx];

        if (std.mem.eql(u8, arg, "--zh")) {
            lang = .zh;
            idx += 1;
            continue;
        }
        if (std.mem.eql(u8, arg, "--en")) {
            lang = .en;
            idx += 1;
            continue;
        }
        if (std.mem.eql(u8, arg, "--lang")) {
            if (idx + 1 >= args.len) {
                std.debug.print("Error: missing value for --lang\n", .{});
                return error.InvalidArguments;
            }
            const parsed = i18n.parseLangValue(args[idx + 1]) orelse {
                std.debug.print("Error: unsupported language: {s}\n", .{args[idx + 1]});
                return error.InvalidArguments;
            };
            lang = parsed;
            idx += 2;
            continue;
        }
        if (std.mem.startsWith(u8, arg, "--lang=")) {
            const raw = arg["--lang=".len..];
            const parsed = i18n.parseLangValue(raw) orelse {
                std.debug.print("Error: unsupported language: {s}\n", .{raw});
                return error.InvalidArguments;
            };
            lang = parsed;
            idx += 1;
            continue;
        }
        break;
    }

    return .{
        .lang = lang,
        .command_index = idx,
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
        try printUsage(allocator, args[0], i18n.detectLangFromEnv(allocator));
        return 1;
    };
    i18n.setForcedLang(global.lang);

    if (global.command_index >= args.len) {
        try printUsage(allocator, args[0], global.lang);
        return 1;
    }

    const first = args[global.command_index];
    const rest = if (global.command_index + 1 < args.len) args[global.command_index + 1 ..] else &.{};
    const selected = classifyCommand(first);
    if (selected == .help) {
        const help_lang: UiLang = if (std.mem.eql(u8, first, "帮助")) .zh else global.lang;
        try printUsage(allocator, args[0], help_lang);
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

test {
    _ = add_repo_cmd;
}

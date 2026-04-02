const std = @import("std");
const i18n = @import("../i18n.zig");

const Allocator = std.mem.Allocator;
const help_en = @embedFile("../i18n/completion_help.en.txt");
const zsh_completion = @embedFile("../completions/gitn.zsh");

fn printUsage(allocator: Allocator, exe_name: []const u8) !void {
    try i18n.printHelpTemplate(allocator, help_en, exe_name);
}

pub fn runWithArgs(allocator: Allocator, args: []const []const u8) !u8 {
    if (args.len == 1) {
        try printUsage(allocator, args[0]);
        return 1;
    }

    if (args.len == 2 and (std.mem.eql(u8, args[1], "-h") or std.mem.eql(u8, args[1], "--help"))) {
        try printUsage(allocator, args[0]);
        return 0;
    }

    if (args.len == 2 and std.mem.eql(u8, args[1], "zsh")) {
        std.debug.print("{s}", .{zsh_completion});
        if (zsh_completion.len == 0 or zsh_completion[zsh_completion.len - 1] != '\n') {
            std.debug.print("\n", .{});
        }
        return 0;
    }

    std.debug.print("Error: unsupported completion target.\n", .{});
    try printUsage(allocator, args[0]);
    return 1;
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

test "zsh completion does not advertise removed language features" {
    const testing = std.testing;

    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "--lang"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "--zh"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "--en"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "帮助"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "诊断"));
}

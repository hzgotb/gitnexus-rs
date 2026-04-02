const std = @import("std");

const Allocator = std.mem.Allocator;

pub fn printHelpTemplate(
    allocator: Allocator,
    template: []const u8,
    exe_name: []const u8,
) !void {
    const rendered = try std.mem.replaceOwned(u8, allocator, template, "{{exe}}", exe_name);
    std.debug.print("{s}", .{rendered});
    if (rendered.len == 0 or rendered[rendered.len - 1] != '\n') {
        std.debug.print("\n", .{});
    }
}

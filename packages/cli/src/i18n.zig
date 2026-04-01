const std = @import("std");

const Allocator = std.mem.Allocator;

pub const UiLang = enum {
    en,
    zh,
};

var forced_lang: ?UiLang = null;

pub fn setForcedLang(lang: ?UiLang) void {
    forced_lang = lang;
}

pub fn parseLangValue(raw_value: []const u8) ?UiLang {
    const value = std.mem.trim(u8, raw_value, " \t\r\n");
    if (value.len == 0) return null;
    if (std.mem.eql(u8, value, "中文")) return .zh;
    if (value.len >= 2 and std.ascii.eqlIgnoreCase(value[0..2], "zh")) return .zh;
    if (value.len >= 2 and std.ascii.eqlIgnoreCase(value[0..2], "en")) return .en;
    return null;
}

pub fn detectLangFromEnv(allocator: Allocator) UiLang {
    if (forced_lang) |lang| return lang;

    const env_names = [_][]const u8{
        "GITN_LANG",
        "LC_ALL",
        "LC_MESSAGES",
        "LANG",
    };
    for (env_names) |name| {
        const value = std.process.getEnvVarOwned(allocator, name) catch continue;
        if (parseLangValue(value)) |lang| return lang;
    }
    return .en;
}

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

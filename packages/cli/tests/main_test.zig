const std = @import("std");
const main_module = @import("cli_main");

test "global language flags are not parsed as CLI-wide options" {
    const testing = std.testing;
    const command_index = try main_module.testing.globalCommandIndex(testing.allocator, &.{
        "gitn",
        "--lang",
        "zh",
        "analyze",
    });

    try testing.expectEqual(@as(usize, 1), command_index);
}

test "Chinese command aliases are no longer built-in commands" {
    const testing = std.testing;

    try testing.expect(main_module.testing.commandIsExternal("帮助"));
    try testing.expect(main_module.testing.commandIsExternal("诊断"));
}

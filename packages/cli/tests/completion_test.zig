const std = @import("std");
const cli = @import("cli_main");
const completion_module = cli.testing.completion_module;

test "zsh completion does not advertise removed language features" {
    const testing = std.testing;
    const zsh_completion = completion_module.testing.zshCompletion();

    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "--lang"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "--zh"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "--en"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "帮助"));
    try testing.expect(!std.mem.containsAtLeast(u8, zsh_completion, 1, "诊断"));
}

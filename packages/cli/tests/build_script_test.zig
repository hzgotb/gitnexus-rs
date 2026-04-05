const std = @import("std");

fn readScript(allocator: std.mem.Allocator, candidates: []const []const u8) ![]u8 {
    for (candidates) |candidate| {
        const script_result = std.fs.cwd().readFileAlloc(allocator, candidate, 16 * 1024);
        if (script_result) |script| {
            return script;
        } else |err| {
            if (err != error.FileNotFound) return err;
        }
    }

    return error.FileNotFound;
}

fn readBuildScript(allocator: std.mem.Allocator) ![]u8 {
    return readScript(allocator, &.{
        "scripts/build-zig.sh",
        "../scripts/build-zig.sh",
        "../../scripts/build-zig.sh",
    });
}

fn readReleaseScript(allocator: std.mem.Allocator) ![]u8 {
    return readScript(allocator, &.{
        "scripts/build_release_all.sh",
        "../scripts/build_release_all.sh",
        "../../scripts/build_release_all.sh",
    });
}

test "scripts/build-zig.sh enters packages/cli before building" {
    const testing = std.testing;
    const build_script = try readBuildScript(testing.allocator);
    defer testing.allocator.free(build_script);

    try testing.expect(std.mem.containsAtLeast(
        u8,
        build_script,
        1,
        "cd \"$ROOT_DIR/packages/cli\"",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        build_script,
        1,
        "--build-file build.zig",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        build_script,
        1,
        "--cache-dir .zig-cache",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        build_script,
        1,
        "--global-cache-dir .zig-global-cache",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        build_script,
        1,
        "--prefix zig-out",
    ));
}

test "scripts/build_release_all.sh reuses packages/cli build.zig" {
    const testing = std.testing;
    const release_script = try readReleaseScript(testing.allocator);
    defer testing.allocator.free(release_script);

    try testing.expect(std.mem.containsAtLeast(
        u8,
        release_script,
        1,
        "cd \"$CLI_DIR\"",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        release_script,
        1,
        "--build-file build.zig",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        release_script,
        1,
        "-Dtarget=\"$target\"",
    ));
    try testing.expect(std.mem.containsAtLeast(
        u8,
        release_script,
        1,
        "-Doptimize=ReleaseSafe",
    ));
    try testing.expect(!std.mem.containsAtLeast(
        u8,
        release_script,
        1,
        "build-exe",
    ));
}

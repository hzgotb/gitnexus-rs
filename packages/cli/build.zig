const std = @import("std");

fn packagePath(b: *std.Build, prefix: []const u8, sub_path: []const u8) std.Build.LazyPath {
    if (prefix.len == 0) return b.path(sub_path);
    return b.path(b.pathJoin(&.{ prefix, sub_path }));
}

fn addExecutableCompat(
    b: *std.Build,
    prefix: []const u8,
    name: []const u8,
    source_path: []const u8,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
) *std.Build.Step.Compile {
    const ExeOptions = std.Build.ExecutableOptions;
    if (@hasField(ExeOptions, "root_source_file")) {
        return b.addExecutable(.{
            .name = name,
            .root_source_file = packagePath(b, prefix, source_path),
            .target = target,
            .optimize = optimize,
        });
    }

    return b.addExecutable(.{
        .name = name,
        .root_module = b.createModule(.{
            .root_source_file = packagePath(b, prefix, source_path),
            .target = target,
            .optimize = optimize,
        }),
    });
}

fn createModuleCompat(
    b: *std.Build,
    prefix: []const u8,
    source_path: []const u8,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
) *std.Build.Module {
    return b.createModule(.{
        .root_source_file = packagePath(b, prefix, source_path),
        .target = target,
        .optimize = optimize,
    });
}

pub fn buildWithPrefix(b: *std.Build, prefix: []const u8) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const gitn_exe = addExecutableCompat(b, prefix, "gitn", "src/main.zig", target, optimize);
    b.installArtifact(gitn_exe);

    const run_gitn_cmd = b.addRunArtifact(gitn_exe);
    if (b.args) |args| {
        run_gitn_cmd.addArgs(args);
    }
    const run_step = b.step("run", "Run gitn");
    run_step.dependOn(&run_gitn_cmd.step);

    const run_gitn_step = b.step("run-gitn", "Run gitn");
    run_gitn_step.dependOn(&run_gitn_cmd.step);

    const cli_tests_root = createModuleCompat(b, prefix, "tests/tests.zig", target, optimize);
    cli_tests_root.addImport("cli_main", createModuleCompat(b, prefix, "src/main.zig", target, optimize));

    const cli_tests = b.addTest(.{
        .name = "cli-tests",
        .root_module = cli_tests_root,
    });
    const run_cli_tests = b.addRunArtifact(cli_tests);
    const test_cli_step = b.step("test-cli", "Run packages/cli tests");
    test_cli_step.dependOn(&run_cli_tests.step);
}

pub fn build(b: *std.Build) void {
    buildWithPrefix(b, "");
}

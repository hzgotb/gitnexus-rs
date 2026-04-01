const std = @import("std");

fn addExecutableCompat(
    b: *std.Build,
    name: []const u8,
    source_path: []const u8,
    target: std.Build.ResolvedTarget,
    optimize: std.builtin.OptimizeMode,
) *std.Build.Step.Compile {
    const ExeOptions = std.Build.ExecutableOptions;
    if (@hasField(ExeOptions, "root_source_file")) {
        return b.addExecutable(.{
            .name = name,
            .root_source_file = b.path(source_path),
            .target = target,
            .optimize = optimize,
        });
    }

    return b.addExecutable(.{
        .name = name,
        .root_module = b.createModule(.{
            .root_source_file = b.path(source_path),
            .target = target,
            .optimize = optimize,
        }),
    });
}

pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});

    const gitn_exe = addExecutableCompat(b, "gitn", "packages/cli/src/main.zig", target, optimize);
    b.installArtifact(gitn_exe);

    const run_gitn_cmd = b.addRunArtifact(gitn_exe);
    if (b.args) |args| {
        run_gitn_cmd.addArgs(args);
    }
    const run_step = b.step("run", "Run gitn");
    run_step.dependOn(&run_gitn_cmd.step);

    const run_gitn_step = b.step("run-gitn", "Run gitn");
    run_gitn_step.dependOn(&run_gitn_cmd.step);
}

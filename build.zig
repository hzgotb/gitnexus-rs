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

    const add_repo_exe = addExecutableCompat(b, "add_repo", "src/add_repo.zig", target, optimize);
    const start_exe = addExecutableCompat(b, "start", "src/start.zig", target, optimize);
    const analyze_exe = addExecutableCompat(b, "analyze", "src/analyze.zig", target, optimize);

    b.installArtifact(add_repo_exe);
    b.installArtifact(start_exe);
    b.installArtifact(analyze_exe);

    const run_start_cmd = b.addRunArtifact(start_exe);
    if (b.args) |args| {
        run_start_cmd.addArgs(args);
    }
    const run_step = b.step("run", "Run start");
    run_step.dependOn(&run_start_cmd.step);

    const run_add_repo_cmd = b.addRunArtifact(add_repo_exe);
    if (b.args) |args| {
        run_add_repo_cmd.addArgs(args);
    }
    const run_add_repo_step = b.step("run-add-repo", "Run add_repo");
    run_add_repo_step.dependOn(&run_add_repo_cmd.step);

    const run_start_step = b.step("run-start", "Run start");
    run_start_step.dependOn(&run_start_cmd.step);

    const run_analyze_cmd = b.addRunArtifact(analyze_exe);
    if (b.args) |args| {
        run_analyze_cmd.addArgs(args);
    }
    const run_analyze_step = b.step("run-analyze", "Run analyze");
    run_analyze_step.dependOn(&run_analyze_cmd.step);
}

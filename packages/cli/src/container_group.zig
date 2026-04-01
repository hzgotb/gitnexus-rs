const std = @import("std");
const container_runtime = @import("container_runtime.zig");

const Allocator = std.mem.Allocator;

pub const registry_label_key = "gitnexus.registry";
pub const repos_label_key = "gitnexus.repos";

pub const JsonGroupPaths = struct {
    registry_json_path: []const u8 = "",
    repos_json_path: []const u8 = "",
};

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn dupeTrimmedOrEmpty(allocator: Allocator, raw: []const u8) ![]const u8 {
    const trimmed = std.mem.trim(u8, raw, " \t\r\n");
    if (trimmed.len == 0) return "";
    return try allocator.dupe(u8, trimmed);
}

pub fn inspectContainerJsonPaths(allocator: Allocator, container_name: []const u8) !JsonGroupPaths {
    const result = try container_runtime.runDocker(allocator, &.{
        "docker",
        "inspect",
        "--format",
        "{{with index .Config.Labels \"gitnexus.registry\"}}{{.}}{{end}}\t{{with index .Config.Labels \"gitnexus.repos\"}}{{.}}{{end}}",
        container_name,
    });
    if (!isExitedZero(result.term)) return error.DockerInspectFailed;

    const line = std.mem.trim(u8, result.stdout, " \t\r\n");
    if (line.len == 0) return .{};

    var cols = std.mem.splitScalar(u8, line, '\t');
    return .{
        .registry_json_path = try dupeTrimmedOrEmpty(allocator, cols.next() orelse ""),
        .repos_json_path = try dupeTrimmedOrEmpty(allocator, cols.next() orelse ""),
    };
}

pub fn appendDockerJsonLabels(
    allocator: Allocator,
    docker_args: *std.ArrayList([]const u8),
    paths: JsonGroupPaths,
) !void {
    if (paths.registry_json_path.len != 0) {
        const registry_label = try std.fmt.allocPrint(allocator, "{s}={s}", .{
            registry_label_key,
            paths.registry_json_path,
        });
        try docker_args.appendSlice(allocator, &.{ "--label", registry_label });
    }
    if (paths.repos_json_path.len != 0) {
        const repos_label = try std.fmt.allocPrint(allocator, "{s}={s}", .{
            repos_label_key,
            paths.repos_json_path,
        });
        try docker_args.appendSlice(allocator, &.{ "--label", repos_label });
    }
}

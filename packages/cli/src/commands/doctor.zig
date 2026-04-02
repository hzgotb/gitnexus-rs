const std = @import("std");
const container_runtime = @import("../container_runtime.zig");
const i18n = @import("../i18n.zig");

const Allocator = std.mem.Allocator;
const JsonValue = std.json.Value;
const max_json_size: usize = 16 * 1024 * 1024;
const help_en = @embedFile("../i18n/doctor_help.en.txt");

pub const CheckOptions = struct {
    create_missing_json: bool = false,
    verbose: bool = false,
    registry_path: ?[]const u8 = null,
    repos_path: ?[]const u8 = null,
};

pub const CheckResult = struct {
    cwd_abs: []const u8,
    registry_json_path: []const u8,
    repos_json_path: []const u8,
};

const JsonKind = enum {
    registry,
    repos,
};

const ManagedContainer = struct {
    name: []const u8,
    status: []const u8,
};

fn printUsage(allocator: Allocator, exe_name: []const u8) !void {
    try i18n.printHelpTemplate(allocator, help_en, exe_name);
}

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn isManagedContainerName(name: []const u8) bool {
    return std.mem.eql(u8, name, "gitnexus") or std.mem.startsWith(u8, name, "gitnexus-");
}

fn printCommandOutput(result: std.process.Child.RunResult) void {
    if (result.stdout.len != 0) {
        std.debug.print("{s}", .{result.stdout});
        if (result.stdout[result.stdout.len - 1] != '\n') {
            std.debug.print("\n", .{});
        }
    }
    if (result.stderr.len != 0) {
        std.debug.print("{s}", .{result.stderr});
        if (result.stderr[result.stderr.len - 1] != '\n') {
            std.debug.print("\n", .{});
        }
    }
}

fn jsonFileName(kind: JsonKind) []const u8 {
    return switch (kind) {
        .registry => "registry.json",
        .repos => "repos.json",
    };
}

fn defaultJsonContent(kind: JsonKind) []const u8 {
    return switch (kind) {
        .registry => "[]\n",
        .repos => "{\n  \"repos\": []\n}\n",
    };
}

fn resolvePathFromCwd(
    allocator: Allocator,
    cwd_abs: []const u8,
    raw_path: []const u8,
) ![]const u8 {
    if (std.fs.path.isAbsolute(raw_path)) {
        return try std.fs.path.resolve(allocator, &.{raw_path});
    }
    return try std.fs.path.resolve(allocator, &.{ cwd_abs, raw_path });
}

fn ensureJsonFileExists(
    path_abs: []const u8,
    kind: JsonKind,
    create_missing: bool,
) !void {
    const existing = std.fs.openFileAbsolute(path_abs, .{}) catch |err| switch (err) {
        error.FileNotFound => {
            if (!create_missing) {
                std.debug.print("Error: missing {s}: {s}\n", .{ jsonFileName(kind), path_abs });
                std.debug.print("Hint: run `gitn doctor --fix` to auto-create it.\n", .{});
                return error.MissingRequiredJsonFile;
            }

            var created = try std.fs.createFileAbsolute(path_abs, .{ .exclusive = true });
            defer created.close();
            try created.writeAll(defaultJsonContent(kind));
            std.debug.print("Info: created missing {s} at {s}\n", .{ jsonFileName(kind), path_abs });
            return;
        },
        else => return err,
    };
    existing.close();
}

fn validateJsonFile(
    allocator: Allocator,
    path_abs: []const u8,
    kind: JsonKind,
) !void {
    const content = std.fs.cwd().readFileAlloc(allocator, path_abs, max_json_size) catch |err| {
        std.debug.print("Error: failed to read {s}: {s}\n", .{ jsonFileName(kind), path_abs });
        return err;
    };
    defer allocator.free(content);

    var parsed = std.json.parseFromSlice(JsonValue, allocator, content, .{
        .allocate = .alloc_always,
    }) catch {
        std.debug.print("Error: {s} is not valid JSON: {s}\n", .{ jsonFileName(kind), path_abs });
        return error.InvalidJsonFile;
    };
    defer parsed.deinit();

    switch (kind) {
        .registry => switch (parsed.value) {
            .array => {},
            else => {
                std.debug.print("Error: {s} root must be a JSON array: {s}\n", .{ jsonFileName(kind), path_abs });
                return error.InvalidJsonFile;
            },
        },
        .repos => {
            const root_obj = switch (parsed.value) {
                .object => |*obj| obj,
                else => {
                    std.debug.print("Error: {s} root must be a JSON object: {s}\n", .{ jsonFileName(kind), path_abs });
                    return error.InvalidJsonFile;
                },
            };

            const repos_value = root_obj.get("repos") orelse {
                std.debug.print("Error: missing 'repos' key in {s}: {s}\n", .{ jsonFileName(kind), path_abs });
                return error.InvalidJsonFile;
            };
            switch (repos_value) {
                .array => {},
                else => {
                    std.debug.print("Error: 'repos' must be an array in {s}: {s}\n", .{ jsonFileName(kind), path_abs });
                    return error.InvalidJsonFile;
                },
            }
        },
    }
}

fn ensureJsonUsable(
    allocator: Allocator,
    path_abs: []const u8,
    kind: JsonKind,
    create_missing: bool,
    verbose: bool,
) !void {
    try ensureJsonFileExists(path_abs, kind, create_missing);
    try validateJsonFile(allocator, path_abs, kind);
    if (verbose) {
        std.debug.print("OK: {s} is usable ({s})\n", .{ jsonFileName(kind), path_abs });
    }
}

fn ensureDockerReady(allocator: Allocator, verbose: bool) !void {
    const probe = try container_runtime.probe(allocator);
    if (!probe.docker_cli_available) {
        std.debug.print("Error: docker CLI is not installed or not in PATH.\n", .{});
        if (probe.colima_installed) {
            std.debug.print("Colima was detected, but gitn still requires the docker CLI.\n", .{});
            std.debug.print("Please install the docker CLI and try again.\n", .{});
        } else {
            std.debug.print("Please install Docker or Colima + docker CLI first.\n", .{});
        }
        return error.DockerNotInstalled;
    }

    const runtime = try container_runtime.resolve(allocator);
    if (runtime.backend == .colima) {
        if (verbose) {
            std.debug.print(
                "OK: container runtime is ready via Colima ({s})\n",
                .{runtime.docker_host.?},
            );
        }
        return;
    }

    if (probe.default_docker_ready) {
        if (verbose) {
            std.debug.print("OK: container runtime is ready via docker\n", .{});
        }
        return;
    }

    std.debug.print("Error: Docker daemon is not running or not reachable.\n", .{});
    if (probe.colima_installed) {
        if (probe.colima_socket_path) |socket_path| {
            std.debug.print(
                "Colima was detected but is not ready. Expected socket: {s}\n",
                .{socket_path},
            );
        } else {
            std.debug.print("Colima was detected but no active docker socket was found.\n", .{});
        }
        std.debug.print("Please run `colima start` (or switch docker context to Colima) and try again.\n", .{});
    } else {
        std.debug.print("Please start Docker and try again.\n", .{});
    }

    const info_result = container_runtime.runDocker(allocator, &.{ "docker", "info", "--format", "{{.ServerVersion}}" }) catch |err| switch (err) {
        error.FileNotFound => return error.DockerNotInstalled,
        else => return err,
    };
    if (!isExitedZero(info_result.term)) {
        printCommandOutput(info_result);
    }
    return error.DockerDaemonUnavailable;
}

fn collectRunningManagedContainers(allocator: Allocator) !std.ArrayList(ManagedContainer) {
    const result = try container_runtime.runDocker(allocator, &.{
        "docker",
        "ps",
        "--format",
        "{{.Names}}\t{{.Status}}",
    });
    if (!isExitedZero(result.term)) {
        printCommandOutput(result);
        return error.DockerPsFailed;
    }

    var containers = std.ArrayList(ManagedContainer).empty;
    var lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;

        var cols = std.mem.splitScalar(u8, line, '\t');
        const name = cols.next() orelse continue;
        const status = cols.next() orelse "";
        if (!isManagedContainerName(name)) continue;

        try containers.append(allocator, .{
            .name = try allocator.dupe(u8, name),
            .status = try allocator.dupe(u8, status),
        });
    }

    return containers;
}

pub fn validateRegistryJsonFile(
    allocator: Allocator,
    path_abs: []const u8,
    verbose: bool,
) !void {
    try ensureJsonUsable(allocator, path_abs, .registry, false, verbose);
}

pub fn validateReposJsonFile(
    allocator: Allocator,
    path_abs: []const u8,
    verbose: bool,
) !void {
    try ensureJsonUsable(allocator, path_abs, .repos, false, verbose);
}

pub fn checkEnvironment(allocator: Allocator, options: CheckOptions) !CheckResult {
    const cwd_abs = try std.fs.cwd().realpathAlloc(allocator, ".");
    const registry_json_path = if (options.registry_path) |path|
        try resolvePathFromCwd(allocator, cwd_abs, path)
    else
        try std.fs.path.join(allocator, &.{ cwd_abs, "registry.json" });
    const repos_json_path = if (options.repos_path) |path|
        try resolvePathFromCwd(allocator, cwd_abs, path)
    else
        try std.fs.path.join(allocator, &.{ cwd_abs, "repos.json" });

    try ensureJsonUsable(
        allocator,
        registry_json_path,
        .registry,
        options.create_missing_json,
        options.verbose,
    );
    try ensureJsonUsable(
        allocator,
        repos_json_path,
        .repos,
        options.create_missing_json,
        options.verbose,
    );
    try ensureDockerReady(allocator, options.verbose);

    return .{
        .cwd_abs = cwd_abs,
        .registry_json_path = registry_json_path,
        .repos_json_path = repos_json_path,
    };
}

pub fn runWithArgs(allocator: Allocator, args: []const []const u8) !u8 {
    var fix = false;
    var registry_override: ?[]const u8 = null;
    var repos_override: ?[]const u8 = null;

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "-h") or std.mem.eql(u8, arg, "--help")) {
            try printUsage(allocator, args[0]);
            return 0;
        }
        if (std.mem.eql(u8, arg, "--fix")) {
            fix = true;
            continue;
        }
        if (std.mem.eql(u8, arg, "-r") or std.mem.eql(u8, arg, "--registry")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                try printUsage(allocator, args[0]);
                return 1;
            }
            const path = std.mem.trim(u8, args[i], " \t\r\n");
            if (path.len == 0) {
                std.debug.print("Error: registry path must not be empty\n", .{});
                return 1;
            }
            registry_override = path;
            continue;
        }
        if (std.mem.eql(u8, arg, "--repos")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                try printUsage(allocator, args[0]);
                return 1;
            }
            const path = std.mem.trim(u8, args[i], " \t\r\n");
            if (path.len == 0) {
                std.debug.print("Error: repos path must not be empty\n", .{});
                return 1;
            }
            repos_override = path;
            continue;
        }

        std.debug.print("Error: unknown argument: {s}\n", .{arg});
        try printUsage(allocator, args[0]);
        return 1;
    }

    const result = try checkEnvironment(allocator, .{
        .create_missing_json = fix,
        .verbose = true,
        .registry_path = registry_override,
        .repos_path = repos_override,
    });
    const runtime = try container_runtime.resolve(allocator);
    const running_containers = try collectRunningManagedContainers(allocator);

    std.debug.print("Doctor checks passed.\n", .{});
    std.debug.print("cwd: {s}\n", .{result.cwd_abs});
    std.debug.print("registry.json: {s}\n", .{result.registry_json_path});
    std.debug.print("repos.json: {s}\n", .{result.repos_json_path});
    std.debug.print("runtime: {s}\n", .{container_runtime.backendLabel(runtime.backend)});
    if (runtime.docker_host) |docker_host| {
        std.debug.print("docker host: {s}\n", .{docker_host});
    }
    std.debug.print("running managed containers: {d}\n", .{running_containers.items.len});
    for (running_containers.items) |container| {
        std.debug.print("  - {s} ({s})\n", .{ container.name, container.status });
    }
    return 0;
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

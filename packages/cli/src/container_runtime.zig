const std = @import("std");
const i18n = @import("i18n.zig");

const Allocator = std.mem.Allocator;
const EnvMap = std.process.EnvMap;
const preferred_runtime_file_name = "runtime.txt";
const runtime_env_name = "GITN_RUNTIME";

pub const Backend = enum {
    docker,
    colima,
};

pub const ResolvedRuntime = struct {
    backend: Backend = .docker,
    docker_host: ?[]const u8 = null,
    env_map: ?*const EnvMap = null,
};

pub const ProbeResult = struct {
    docker_cli_available: bool = false,
    default_docker_ready: bool = false,
    colima_installed: bool = false,
    colima_socket_path: ?[]const u8 = null,
    colima_ready: bool = false,
};

var cached_runtime: ?ResolvedRuntime = null;
var runtime_cached = false;

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn runChild(
    allocator: Allocator,
    argv: []const []const u8,
    env_map: ?*const EnvMap,
) !std.process.Child.RunResult {
    return std.process.Child.run(.{
        .allocator = allocator,
        .argv = argv,
        .env_map = env_map,
    });
}

fn commandExists(allocator: Allocator, argv: []const []const u8) !bool {
    _ = std.process.Child.run(.{
        .allocator = allocator,
        .argv = argv,
    }) catch |err| switch (err) {
        error.FileNotFound => return false,
        else => return err,
    };
    return true;
}

fn getEnvVarOwnedOrEmpty(allocator: Allocator, name: []const u8) ![]const u8 {
    return std.process.getEnvVarOwned(allocator, name) catch |err| switch (err) {
        error.EnvironmentVariableNotFound => "",
        else => return err,
    };
}

fn parseBackend(raw: []const u8) ?Backend {
    const trimmed = std.mem.trim(u8, raw, " \t\r\n");
    if (trimmed.len == 0) return null;
    if (std.ascii.eqlIgnoreCase(trimmed, "docker")) return .docker;
    if (std.ascii.eqlIgnoreCase(trimmed, "colima")) return .colima;
    return null;
}

fn readLineAlloc(allocator: Allocator) ![]u8 {
    var stdin_file = std.fs.File.stdin();
    var buf = std.ArrayList(u8).empty;

    while (true) {
        var byte: [1]u8 = undefined;
        const n = try stdin_file.read(&byte);
        if (n == 0) break;
        if (byte[0] == '\n') break;
        if (byte[0] == '\r') continue;
        try buf.append(allocator, byte[0]);
    }

    return try buf.toOwnedSlice(allocator);
}

fn canPromptForChoice() bool {
    return std.fs.File.stdin().isTty() and std.fs.File.stdout().isTty();
}

fn getPreferenceFilePath(allocator: Allocator) !?[]const u8 {
    const app_data_dir = std.fs.getAppDataDir(allocator, "gitn") catch |err| switch (err) {
        error.AppDataDirUnavailable => return null,
        else => return err,
    };
    return try std.fs.path.join(allocator, &.{ app_data_dir, preferred_runtime_file_name });
}

fn loadPreferredBackendFromFile(allocator: Allocator) !?Backend {
    const path = try getPreferenceFilePath(allocator) orelse return null;
    const content = std.fs.cwd().readFileAlloc(allocator, path, 64) catch |err| switch (err) {
        error.FileNotFound => return null,
        else => return err,
    };
    return parseBackend(content);
}

fn savePreferredBackend(allocator: Allocator, backend: Backend) !void {
    const path = try getPreferenceFilePath(allocator) orelse return;
    const dir_path = std.fs.path.dirname(path) orelse return;
    try std.fs.cwd().makePath(dir_path);

    var file = try std.fs.createFileAbsolute(path, .{ .truncate = true });
    defer file.close();
    try file.writeAll(backendLabel(backend));
    try file.writeAll("\n");
}

fn selectPreferredBackendInteractive(allocator: Allocator) !Backend {
    const lang = i18n.detectLangFromEnv(allocator);
    switch (lang) {
        .zh => {
            std.debug.print("检测到多个可用容器运行时：\n", .{});
            std.debug.print("  [1] docker\n", .{});
            std.debug.print("  [2] colima\n", .{});
            std.debug.print("请选择 gitn 默认使用的运行时（默认 1）: ", .{});
        },
        .en => {
            std.debug.print("Multiple container runtimes are available:\n", .{});
            std.debug.print("  [1] docker\n", .{});
            std.debug.print("  [2] colima\n", .{});
            std.debug.print("Select the default runtime for gitn (default 1): ", .{});
        },
    }

    const input = try readLineAlloc(allocator);
    const trimmed = std.mem.trim(u8, input, " \t\r\n");
    if (trimmed.len == 0) return .docker;
    if (std.mem.eql(u8, trimmed, "2")) return .colima;
    return parseBackend(trimmed) orelse .docker;
}

fn createRuntimeFromProbe(
    allocator: Allocator,
    probe_result: ProbeResult,
    backend: Backend,
) !ResolvedRuntime {
    var runtime = ResolvedRuntime{ .backend = backend };
    switch (backend) {
        .docker => return runtime,
        .colima => {
            const socket_path = probe_result.colima_socket_path orelse return runtime;
            const docker_host = try std.fmt.allocPrint(allocator, "unix://{s}", .{socket_path});
            const env_map = try buildEnvMapWithDockerHost(allocator, docker_host);
            runtime.docker_host = docker_host;
            runtime.env_map = env_map;
            return runtime;
        },
    }
}

fn detectColimaBaseDir(allocator: Allocator) ![]const u8 {
    const colima_home = try getEnvVarOwnedOrEmpty(allocator, "COLIMA_HOME");
    if (colima_home.len != 0) return colima_home;

    const home = try getEnvVarOwnedOrEmpty(allocator, "HOME");
    if (home.len == 0) return "";

    return try std.fs.path.join(allocator, &.{ home, ".colima" });
}

fn detectColimaSocketPath(allocator: Allocator) !?[]const u8 {
    const base_dir = try detectColimaBaseDir(allocator);
    if (base_dir.len == 0) return null;

    const profile_raw = try getEnvVarOwnedOrEmpty(allocator, "COLIMA_PROFILE");
    const profile = if (profile_raw.len != 0) profile_raw else "default";
    const socket_path = try std.fs.path.join(allocator, &.{ base_dir, profile, "docker.sock" });

    std.posix.access(socket_path, 0) catch |err| switch (err) {
        error.FileNotFound => return null,
        error.AccessDenied, error.PermissionDenied => return null,
        else => return null,
    };
    return socket_path;
}

fn buildEnvMapWithDockerHost(allocator: Allocator, docker_host: []const u8) !*EnvMap {
    var env_map = try allocator.create(EnvMap);
    env_map.* = try std.process.getEnvMap(allocator);
    try env_map.put("DOCKER_HOST", docker_host);
    return env_map;
}

fn tryDockerInfo(
    allocator: Allocator,
    env_map: ?*const EnvMap,
) !bool {
    const result = try runChild(allocator, &.{ "docker", "info", "--format", "{{.ServerVersion}}" }, env_map);
    return isExitedZero(result.term);
}

pub fn probe(allocator: Allocator) !ProbeResult {
    var result = ProbeResult{};

    result.docker_cli_available = try commandExists(allocator, &.{ "docker", "--version" });
    if (!result.docker_cli_available) {
        result.colima_installed = try commandExists(allocator, &.{ "colima", "version" });
        return result;
    }

    result.default_docker_ready = tryDockerInfo(allocator, null) catch |err| switch (err) {
        error.FileNotFound => false,
        else => return err,
    };

    result.colima_installed = try commandExists(allocator, &.{ "colima", "version" });
    if (!result.colima_installed) return result;

    result.colima_socket_path = try detectColimaSocketPath(allocator);
    if (result.colima_socket_path) |socket_path| {
        const docker_host = try std.fmt.allocPrint(allocator, "unix://{s}", .{socket_path});
        const env_map = try buildEnvMapWithDockerHost(allocator, docker_host);
        result.colima_ready = tryDockerInfo(allocator, env_map) catch |err| switch (err) {
            error.FileNotFound => false,
            else => return err,
        };
    }

    return result;
}

pub fn resolve(allocator: Allocator) !ResolvedRuntime {
    if (runtime_cached) return cached_runtime orelse .{};

    var runtime = ResolvedRuntime{};
    const probe_result = try probe(allocator);
    if (!probe_result.docker_cli_available) {
        cached_runtime = runtime;
        runtime_cached = true;
        return runtime;
    }

    const env_choice = parseBackend(try getEnvVarOwnedOrEmpty(allocator, runtime_env_name));
    const saved_choice = try loadPreferredBackendFromFile(allocator);
    const preferred_choice = env_choice orelse saved_choice;

    if (preferred_choice) |backend| {
        if (backend == .docker and probe_result.default_docker_ready) {
            runtime = try createRuntimeFromProbe(allocator, probe_result, .docker);
            cached_runtime = runtime;
            runtime_cached = true;
            return runtime;
        }
        if (backend == .colima and probe_result.colima_ready) {
            runtime = try createRuntimeFromProbe(allocator, probe_result, .colima);
            cached_runtime = runtime;
            runtime_cached = true;
            return runtime;
        }
    }

    if (probe_result.default_docker_ready and !probe_result.colima_ready) {
        runtime = try createRuntimeFromProbe(allocator, probe_result, .docker);
        cached_runtime = runtime;
        runtime_cached = true;
        return runtime;
    }
    if (!probe_result.default_docker_ready and probe_result.colima_ready) {
        runtime = try createRuntimeFromProbe(allocator, probe_result, .colima);
        cached_runtime = runtime;
        runtime_cached = true;
        return runtime;
    }
    if (probe_result.default_docker_ready and probe_result.colima_ready) {
        const chosen_backend = preferred_choice orelse blk: {
            if (canPromptForChoice()) {
                const selected = try selectPreferredBackendInteractive(allocator);
                try savePreferredBackend(allocator, selected);
                switch (i18n.detectLangFromEnv(allocator)) {
                    .zh => std.debug.print("已保存默认运行时: {s}\n", .{backendLabel(selected)}),
                    .en => std.debug.print("Saved default runtime: {s}\n", .{backendLabel(selected)}),
                }
                break :blk selected;
            }
            break :blk .docker;
        };
        runtime = try createRuntimeFromProbe(allocator, probe_result, chosen_backend);
    }

    cached_runtime = runtime;
    runtime_cached = true;
    return runtime;
}

pub fn backendLabel(backend: Backend) []const u8 {
    return switch (backend) {
        .docker => "docker",
        .colima => "colima",
    };
}

pub fn runDocker(
    allocator: Allocator,
    argv: []const []const u8,
) !std.process.Child.RunResult {
    const runtime = try resolve(allocator);
    return runChild(allocator, argv, runtime.env_map);
}

pub fn initDockerChild(
    allocator: Allocator,
    argv: []const []const u8,
) !std.process.Child {
    const runtime = try resolve(allocator);
    var child = std.process.Child.init(argv, allocator);
    child.env_map = runtime.env_map;
    return child;
}

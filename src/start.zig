const builtin = @import("builtin");
const std = @import("std");

const Allocator = std.mem.Allocator;
const JsonValue = std.json.Value;

const image_repo = "gitnexus";
const default_tag = "latest";
const container_name_prefix = "gitnexus";
const default_port = "4747";

const LaunchMode = enum {
    create_new,
    rebuild_existing,
    cancel,
};

const OldContainer = struct {
    id: []const u8,
    name: []const u8,
    status: []const u8,
};

fn printUsage(exe_name: []const u8) void {
    std.debug.print(
        \\Usage:
        \\  {s} [--tag <tag>] [--port <port>] [--registry <path>]
        \\
        \\Options:
        \\  -h, --help    Show this help message
        \\  -t, --tag     Override image tag (default: v1)
        \\  -p, --port    Bind host port directly (1-65535), skip port prompt
        \\  -r, --registry  Override registry.json path (absolute or relative)
        \\
    ,
        .{exe_name},
    );
}

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn generateContainerName(allocator: Allocator) ![]const u8 {
    const rand_int = std.crypto.random.int(u32);
    return try std.fmt.allocPrint(allocator, "{s}-{x}", .{ container_name_prefix, rand_int });
}

fn isManagedContainerName(name: []const u8) bool {
    return std.mem.eql(u8, name, container_name_prefix) or std.mem.startsWith(u8, name, container_name_prefix ++ "-");
}

fn containsContainerName(containers: []const OldContainer, name: []const u8) bool {
    for (containers) |container| {
        if (std.mem.eql(u8, container.name, name)) return true;
    }
    return false;
}

fn generateUniqueContainerName(allocator: Allocator, existing: []const OldContainer) ![]const u8 {
    var tries: usize = 0;
    while (tries < 64) : (tries += 1) {
        const name = try generateContainerName(allocator);
        if (!containsContainerName(existing, name)) return name;
    }
    return error.FailedToGenerateUniqueContainerName;
}

fn collectOldContainers(allocator: Allocator) !std.ArrayList(OldContainer) {
    const ps_result = try runDocker(allocator, &.{
        "docker",
        "ps",
        "-a",
        "--format",
        "{{.ID}}\t{{.Names}}\t{{.Status}}",
    });
    if (!isExitedZero(ps_result.term)) {
        std.debug.print("Error: failed to list docker containers.\n", .{});
        printCommandOutput(ps_result);
        return error.DockerPsFailed;
    }

    var result = std.ArrayList(OldContainer).empty;
    var lines = std.mem.splitScalar(u8, ps_result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;

        var cols = std.mem.splitScalar(u8, line, '\t');
        const id = cols.next() orelse continue;
        const name = cols.next() orelse continue;
        const status = cols.next() orelse "";
        if (!isManagedContainerName(name)) continue;

        try result.append(allocator, .{
            .id = try allocator.dupe(u8, id),
            .name = try allocator.dupe(u8, name),
            .status = try allocator.dupe(u8, status),
        });
    }

    return result;
}

fn isAllDigits(s: []const u8) bool {
    if (s.len == 0) return false;
    for (s) |c| {
        if (!std.ascii.isDigit(c)) return false;
    }
    return true;
}

fn isValidPort(port_text: []const u8) bool {
    if (!isAllDigits(port_text)) return false;
    const parsed = std.fmt.parseInt(u16, port_text, 10) catch return false;
    return parsed >= 1;
}

fn detectHomeDir(allocator: Allocator) ![]const u8 {
    const home = std.process.getEnvVarOwned(allocator, "HOME") catch |err| switch (err) {
        error.EnvironmentVariableNotFound => "",
        else => return err,
    };
    if (home.len != 0) return home;

    const user_profile = std.process.getEnvVarOwned(allocator, "USERPROFILE") catch |err| switch (err) {
        error.EnvironmentVariableNotFound => "",
        else => return err,
    };
    if (user_profile.len != 0) return user_profile;

    return "";
}

fn resolveRepoHostPath(
    allocator: Allocator,
    cwd_abs: []const u8,
    home_dir: []const u8,
    raw_path: []const u8,
) ![]const u8 {
    const trimmed = std.mem.trim(u8, raw_path, " \t\r\n");
    if (trimmed.len == 0) return error.InvalidPath;

    var candidate: []const u8 = trimmed;
    if (home_dir.len != 0) {
        if (std.mem.eql(u8, trimmed, "~")) {
            candidate = home_dir;
        } else if (std.mem.startsWith(u8, trimmed, "~/") or std.mem.startsWith(u8, trimmed, "~\\")) {
            candidate = try std.fs.path.join(allocator, &.{ home_dir, trimmed[2..] });
        }
    }

    if (std.fs.path.isAbsolute(candidate)) {
        return try std.fs.path.resolve(allocator, &.{candidate});
    }
    return try std.fs.path.resolve(allocator, &.{ cwd_abs, candidate });
}

fn isValidRepoDirName(name: []const u8) bool {
    if (name.len == 0) return false;
    if (std.mem.eql(u8, name, ".") or std.mem.eql(u8, name, "..")) return false;
    if (std.mem.indexOfScalar(u8, name, '/')) |_| return false;
    if (std.mem.indexOfScalar(u8, name, '\\')) |_| return false;
    return true;
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

fn promptLine(allocator: Allocator, prompt: []const u8) ![]const u8 {
    std.debug.print("{s}", .{prompt});
    return try readLineAlloc(allocator);
}

fn runCommandInherit(allocator: Allocator, argv: []const []const u8) !std.process.Child.Term {
    var child = std.process.Child.init(argv, allocator);
    child.stdin_behavior = .Inherit;
    child.stdout_behavior = .Inherit;
    child.stderr_behavior = .Inherit;
    try child.spawn();
    return try child.wait();
}

fn getSttyState(allocator: Allocator) ![]const u8 {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;

    var child = std.process.Child.init(&.{ "stty", "-g" }, allocator);
    child.stdin_behavior = .Inherit;
    child.stdout_behavior = .Pipe;
    child.stderr_behavior = .Ignore;
    try child.spawn();

    const stdout = try child.stdout.?.deprecatedReader().readAllAlloc(allocator, 1024);
    const term = try child.wait();
    if (!isExitedZero(term)) return error.SttyFailed;

    return std.mem.trim(u8, stdout, " \t\r\n");
}

fn setSttyRawNoEcho(allocator: Allocator) !void {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;
    const term = try runCommandInherit(allocator, &.{ "stty", "raw", "-echo" });
    if (!isExitedZero(term)) return error.SttyFailed;
}

fn restoreStty(allocator: Allocator, state: []const u8) !void {
    if (builtin.os.tag == .windows) return;
    var argv = [_][]const u8{ "stty", state };
    const term = try runCommandInherit(allocator, &argv);
    if (!isExitedZero(term)) return error.SttyFailed;
}

fn selectContainerByPrompt(allocator: Allocator, containers: []const OldContainer) !usize {
    const select_prompt = try std.fmt.allocPrint(
        allocator,
        "Select container index to rebuild (1-{d}, default 1): ",
        .{containers.len},
    );
    const select_input = try promptLine(allocator, select_prompt);
    const select_trimmed = std.mem.trim(u8, select_input, " \t\r\n");

    var selected_index: usize = 0;
    if (select_trimmed.len != 0 and isAllDigits(select_trimmed)) {
        const parsed = std.fmt.parseInt(usize, select_trimmed, 10) catch 1;
        if (parsed >= 1 and parsed <= containers.len) {
            selected_index = parsed - 1;
        }
    }
    return selected_index;
}

fn selectContainerByArrow(allocator: Allocator, containers: []const OldContainer) !usize {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;
    if (!std.fs.File.stdin().isTty()) return error.NotATerminal;
    if (containers.len == 0) return error.EmptyContainerList;

    const saved_stty = try getSttyState(allocator);
    try setSttyRawNoEcho(allocator);
    std.debug.print("\x1b[?25l", .{});
    defer {
        std.debug.print("\x1b[?25h", .{});
        restoreStty(allocator, saved_stty) catch {};
        std.debug.print("\r\n", .{});
    }

    var selected: usize = 0;
    var rendered_once = false;
    var stdin_file = std.fs.File.stdin();

    while (true) {
        if (rendered_once) {
            std.debug.print("\x1b[{d}A", .{containers.len + 1});
        } else {
            rendered_once = true;
        }

        std.debug.print("\x1b[2K\rUse Up/Down arrows, Enter to confirm\n", .{});
        for (containers, 0..) |old, idx| {
            const prefix = if (idx == selected) "> " else "  ";
            std.debug.print("\x1b[2K\r{s}[{d}] {s} ({s})\n", .{ prefix, idx + 1, old.name, old.status });
        }

        var key: [1]u8 = undefined;
        const n = try stdin_file.read(&key);
        if (n == 0) break;

        switch (key[0]) {
            '\n', '\r' => break,
            'k', 'K' => {
                if (selected > 0) selected -= 1;
            },
            'j', 'J' => {
                if (selected + 1 < containers.len) selected += 1;
            },
            27 => {
                var seq: [2]u8 = undefined;
                const n1 = try stdin_file.read(seq[0..1]);
                if (n1 == 0) continue;
                const n2 = try stdin_file.read(seq[1..2]);
                if (n2 == 0) continue;

                if (seq[0] == '[' and seq[1] == 'A') {
                    if (selected > 0) selected -= 1;
                } else if (seq[0] == '[' and seq[1] == 'B') {
                    if (selected + 1 < containers.len) selected += 1;
                }
            },
            else => {},
        }
    }

    return selected;
}

fn selectContainerIndex(allocator: Allocator, containers: []const OldContainer) !usize {
    return selectContainerByArrow(allocator, containers) catch {
        return try selectContainerByPrompt(allocator, containers);
    };
}

fn runDocker(allocator: Allocator, argv: []const []const u8) !std.process.Child.RunResult {
    return std.process.Child.run(.{
        .allocator = allocator,
        .argv = argv,
    });
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

fn run(allocator: Allocator) !u8 {
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    var image_tag: []const u8 = default_tag;
    var port_override: ?[]const u8 = null;
    var registry_override: ?[]const u8 = null;
    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "-h") or std.mem.eql(u8, arg, "--help")) {
            printUsage(args[0]);
            return 0;
        }
        if (std.mem.eql(u8, arg, "-t") or std.mem.eql(u8, arg, "--tag")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                printUsage(args[0]);
                return 1;
            }
            const tag = std.mem.trim(u8, args[i], " \t\r\n");
            if (tag.len == 0) {
                std.debug.print("Error: image tag must not be empty\n", .{});
                return 1;
            }
            image_tag = tag;
            continue;
        }
        if (std.mem.eql(u8, arg, "-p") or std.mem.eql(u8, arg, "--port")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                printUsage(args[0]);
                return 1;
            }
            const port = std.mem.trim(u8, args[i], " \t\r\n");
            if (!isValidPort(port)) {
                std.debug.print("Error: invalid port '{s}', expected 1-65535\n", .{port});
                return 1;
            }
            port_override = port;
            continue;
        }
        if (std.mem.eql(u8, arg, "-r") or std.mem.eql(u8, arg, "--registry")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                printUsage(args[0]);
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

        std.debug.print("Error: unknown argument: {s}\n", .{arg});
        printUsage(args[0]);
        return 1;
    }

    const image_name = try std.fmt.allocPrint(allocator, "{s}:{s}", .{ image_repo, image_tag });

    const cwd_abs = try std.fs.cwd().realpathAlloc(allocator, ".");
    const registry_file = if (registry_override) |path|
        try resolvePathFromCwd(allocator, cwd_abs, path)
    else
        try std.fs.path.join(allocator, &.{ cwd_abs, "registry.json" });
    const repos_json = try std.fs.path.join(allocator, &.{ cwd_abs, "repos.json" });

    {
        const file = std.fs.openFileAbsolute(registry_file, .{}) catch {
            std.debug.print("Error: missing config file: {s}\n", .{registry_file});
            return 1;
        };
        file.close();
    }

    const repos_content = std.fs.cwd().readFileAlloc(allocator, repos_json, 16 * 1024 * 1024) catch {
        std.debug.print("Error: missing repos list file: {s}\n", .{repos_json});
        return 1;
    };
    var repos_parsed = std.json.parseFromSlice(JsonValue, allocator, repos_content, .{
        .allocate = .alloc_always,
    }) catch {
        std.debug.print("Error: repos.json is not valid JSON: {s}\n", .{repos_json});
        return 1;
    };
    defer repos_parsed.deinit();

    const repos_root_obj = switch (repos_parsed.value) {
        .object => |*obj| obj,
        else => {
            std.debug.print("Error: repos.json root must be an object: {s}\n", .{repos_json});
            return 1;
        },
    };
    const repos_value = repos_root_obj.get("repos") orelse {
        std.debug.print("Error: missing 'repos' key in {s}\n", .{repos_json});
        return 1;
    };
    const repos_array = switch (repos_value) {
        .array => |arr| arr,
        else => {
            std.debug.print("Error: 'repos' must be an array in {s}\n", .{repos_json});
            return 1;
        },
    };

    const home_dir = try detectHomeDir(allocator);
    var repo_mounts = std.ArrayList([]const u8).empty;
    var used_dest_names = std.StringHashMap(void).init(allocator);
    for (repos_array.items, 0..) |entry, idx| {
        const arr = switch (entry) {
            .array => |arr| arr,
            else => {
                std.debug.print("Error: repos[{d}] must be an array [host_path, dest_name]\n", .{idx});
                return 1;
            },
        };

        if (arr.items.len < 2) {
            std.debug.print("Error: repos[{d}] must contain host path and dest name\n", .{idx});
            return 1;
        }

        const host_raw = switch (arr.items[0]) {
            .string => |s| s,
            else => {
                std.debug.print("Error: repos[{d}][0] must be a string host path\n", .{idx});
                return 1;
            },
        };
        const dest_raw = switch (arr.items[1]) {
            .string => |s| s,
            else => {
                std.debug.print("Error: repos[{d}][1] must be a string container dir name\n", .{idx});
                return 1;
            },
        };

        const host_trimmed = std.mem.trim(u8, host_raw, " \t\r\n");
        const dest_trimmed = std.mem.trim(u8, dest_raw, " \t\r\n");
        if (host_trimmed.len == 0) {
            std.debug.print("Error: repos[{d}][0] host path must not be empty\n", .{idx});
            return 1;
        }
        if (!isValidRepoDirName(dest_trimmed)) {
            std.debug.print("Error: invalid repos[{d}][1] dest name: '{s}'\n", .{ idx, dest_trimmed });
            return 1;
        }

        const dest_entry = try used_dest_names.getOrPut(dest_trimmed);
        if (dest_entry.found_existing) {
            std.debug.print("Error: duplicate destination name in repos.json: '{s}'\n", .{dest_trimmed});
            return 1;
        }

        const host_abs = resolveRepoHostPath(allocator, cwd_abs, home_dir, host_trimmed) catch {
            std.debug.print("Error: invalid host path for repos[{d}]: {s}\n", .{ idx, host_trimmed });
            return 1;
        };
        var host_dir = std.fs.openDirAbsolute(host_abs, .{}) catch {
            std.debug.print("Error: host repo path does not exist or is not directory: {s}\n", .{host_abs});
            return 1;
        };
        host_dir.close();

        const mount = try std.fmt.allocPrint(allocator, "{s}:/repos/{s}", .{ host_abs, dest_trimmed });
        try repo_mounts.append(allocator, mount);
    }

    const old_containers = collectOldContainers(allocator) catch |err| switch (err) {
        error.DockerPsFailed => return 1,
        else => return err,
    };

    var launch_mode: LaunchMode = .create_new;
    var rebuild_target: ?OldContainer = null;
    if (old_containers.items.len != 0) {
        std.debug.print("Found existing containers:\n", .{});
        for (old_containers.items, 0..) |old, idx| {
            std.debug.print("  [{d}] {s} ({s})\n", .{ idx + 1, old.name, old.status });
        }
        std.debug.print("Choose mode:\n", .{});
        std.debug.print("  [n] new container\n", .{});
        std.debug.print("  [r] rebuild existing\n", .{});
        std.debug.print("  [c] cancel\n", .{});
        const mode_input = try promptLine(allocator, "Your choice (default n): ");
        const mode_trimmed = std.mem.trim(u8, mode_input, " \t\r\n");
        if (mode_trimmed.len != 0 and std.ascii.toLower(mode_trimmed[0]) == 'r') {
            launch_mode = .rebuild_existing;
            const selected_index = try selectContainerIndex(allocator, old_containers.items);
            rebuild_target = old_containers.items[selected_index];
            std.debug.print("Rebuilding container: {s}\n", .{rebuild_target.?.name});
        } else if (mode_trimmed.len != 0 and std.ascii.toLower(mode_trimmed[0]) == 'c') {
            launch_mode = .cancel;
        } else if (mode_trimmed.len != 0 and std.ascii.toLower(mode_trimmed[0]) != 'n') {
            std.debug.print("Warning: invalid mode input, fallback to new container.\n", .{});
        }
    }

    if (launch_mode == .cancel) {
        std.debug.print("Canceled by user.\n", .{});
        return 0;
    }

    const container_name = switch (launch_mode) {
        .create_new => try generateUniqueContainerName(allocator, old_containers.items),
        .rebuild_existing => rebuild_target.?.name,
        .cancel => unreachable,
    };

    var target_port: []const u8 = "";
    if (port_override) |port| {
        target_port = port;
        std.debug.print("Port selected via argument: {s}\n", .{port});
    } else if (launch_mode == .rebuild_existing) {
        const inspect_result = try runDocker(allocator, &.{
            "docker",
            "inspect",
            "--format={{range $p, $conf := .HostConfig.PortBindings}}{{(index $conf 0).HostPort}}{{break}}{{end}}",
            rebuild_target.?.name,
        });
        const old_port = if (isExitedZero(inspect_result.term))
            std.mem.trim(u8, inspect_result.stdout, " \t\r\n")
        else
            "";

        if (old_port.len != 0) {
            std.debug.print("Existing host port: {s}\n", .{old_port});
            std.debug.print("Choose: [y] reuse old port | [number] set new port | [n] no binding\n", .{});
            const user_input = try promptLine(allocator, "Your choice (default y): ");
            const trimmed = std.mem.trim(u8, user_input, " \t\r\n");
            const effective_input = if (trimmed.len == 0) "y" else trimmed;

            const lowered = try allocator.dupe(u8, effective_input);
            for (lowered) |*c| c.* = std.ascii.toLower(c.*);

            if (std.mem.eql(u8, lowered, "y")) {
                target_port = old_port;
            } else if (std.mem.eql(u8, lowered, "n")) {
                target_port = "";
            } else if (isAllDigits(effective_input)) {
                target_port = effective_input;
            } else {
                std.debug.print("Warning: invalid input; fallback to old port ({s}).\n", .{old_port});
                target_port = old_port;
            }
        } else {
            std.debug.print("Container has no bound host port.\n", .{});
            const user_input = try promptLine(allocator, "Input host port (empty to skip): ");
            target_port = std.mem.trim(u8, user_input, " \t\r\n");
        }
    } else {
        const user_input = try promptLine(allocator, "Input host port to bind (empty to skip): ");
        target_port = std.mem.trim(u8, user_input, " \t\r\n");
    }

    if (launch_mode == .rebuild_existing) {
        const rm_result = try runDocker(allocator, &.{ "docker", "rm", "-f", rebuild_target.?.id });
        if (!isExitedZero(rm_result.term)) {
            std.debug.print("Error: failed to remove old container before rebuild.\n", .{});
            printCommandOutput(rm_result);
            return 1;
        }
    }

    var docker_args = std.ArrayList([]const u8).empty;
    try docker_args.appendSlice(allocator, &.{ "docker", "run", "-d" });
    try docker_args.appendSlice(allocator, &.{ "--name", container_name });

    const registry_mount = try std.fmt.allocPrint(allocator, "{s}:/root/.gitnexus/registry.json", .{registry_file});
    try docker_args.appendSlice(allocator, &.{ "-v", registry_mount });
    for (repo_mounts.items) |mount| {
        try docker_args.appendSlice(allocator, &.{ "-v", mount });
    }
    try docker_args.appendSlice(allocator, &.{ "--restart", "unless-stopped" });

    if (target_port.len != 0) {
        const port_map = try std.fmt.allocPrint(allocator, "{s}:{s}", .{ target_port, target_port });
        const serve_port_env = try std.fmt.allocPrint(allocator, "SERVE_PORT={s}", .{target_port});
        try docker_args.appendSlice(allocator, &.{ "-p", port_map, "-e", serve_port_env });
        std.debug.print("Port config: host {s} -> container {s}\n", .{ target_port, target_port });
    } else {
        const serve_port_env = "SERVE_PORT=" ++ default_port;
        try docker_args.appendSlice(allocator, &.{ "-e", serve_port_env });
        std.debug.print("Port config: no host binding, container listens on {s}\n", .{default_port});
    }

    try docker_args.append(allocator, image_name);

    std.debug.print("Starting image {s} with container {s}...\n", .{ image_name, container_name });
    const run_result = try runDocker(allocator, docker_args.items);
    printCommandOutput(run_result);
    if (!isExitedZero(run_result.term)) {
        std.debug.print("Error: failed to start container.\n", .{});
        return 1;
    }

    std.debug.print("-----------------------------------------------\n", .{});
    std.debug.print("Started successfully.\n", .{});
    if (target_port.len != 0) {
        std.debug.print("Access URL: http://localhost:{s}\n", .{target_port});
    } else {
        std.debug.print("Access mode: Docker-internal networking only.\n", .{});
    }
    std.debug.print("Logs: docker logs -f {s}\n", .{container_name});
    std.debug.print("-----------------------------------------------\n", .{});

    return 0;
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

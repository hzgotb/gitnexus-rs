const builtin = @import("builtin");
const std = @import("std");

const Allocator = std.mem.Allocator;

const container_name_prefix = "gitnexus";

const Container = struct {
    id: []const u8,
    name: []const u8,
    status: []const u8,
};

const ActiveAnalyze = struct {
    repo: []const u8,
    child: std.process.Child,
};

fn printUsage(exe_name: []const u8) void {
    std.debug.print(
        \\Usage:
        \\  {s} [--container <name-or-id>] [--repo <repo_name> | --all] [--jobs <n>] [--] [analyze_args...]
        \\
        \\Options:
        \\  -h, --help       Show this help message
        \\  -c, --container  Specify target running container
        \\  -r, --repo       Specify target repo under /repos in container
        \\  -a, --all        Run analyze for all repos under /repos
        \\  -j, --jobs       Parallel jobs for --all (default: 1)
        \\
        \\Examples:
        \\  {s}                         # interactive choose container/repo
        \\  {s} --all                   # run analyze for every repo
        \\  {s} --all -j 4              # run analyze concurrently in 4 repos
        \\  {s} -c gitnexus-abcd1234    # choose repo in container
        \\  {s} -c gitnexus-abcd1234 -r vue -- --format json
        \\  {s} -c gitnexus-abcd1234 --all -- --format json
        \\
    ,
        .{ exe_name, exe_name, exe_name, exe_name, exe_name, exe_name, exe_name },
    );
}

fn isExitedZero(term: std.process.Child.Term) bool {
    return switch (term) {
        .Exited => |code| code == 0,
        else => false,
    };
}

fn exitCodeFromTerm(term: std.process.Child.Term) u8 {
    return switch (term) {
        .Exited => |code| @as(u8, @intCast(if (code > 255) 1 else code)),
        else => 1,
    };
}

fn isManagedContainerName(name: []const u8) bool {
    return std.mem.eql(u8, name, container_name_prefix) or std.mem.startsWith(u8, name, container_name_prefix ++ "-");
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

fn runDocker(allocator: Allocator, argv: []const []const u8) !std.process.Child.RunResult {
    return std.process.Child.run(.{
        .allocator = allocator,
        .argv = argv,
    });
}

fn collectRunningContainers(allocator: Allocator) !std.ArrayList(Container) {
    const result = try runDocker(allocator, &.{
        "docker",
        "ps",
        "--format",
        "{{.ID}}\t{{.Names}}\t{{.Status}}",
    });
    if (!isExitedZero(result.term)) {
        std.debug.print("Error: failed to list running containers.\n", .{});
        printCommandOutput(result);
        return error.DockerPsFailed;
    }

    var containers = std.ArrayList(Container).empty;
    var lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;

        var cols = std.mem.splitScalar(u8, line, '\t');
        const id = cols.next() orelse continue;
        const name = cols.next() orelse continue;
        const status = cols.next() orelse "";
        if (!isManagedContainerName(name)) continue;

        try containers.append(allocator, .{
            .id = try allocator.dupe(u8, id),
            .name = try allocator.dupe(u8, name),
            .status = try allocator.dupe(u8, status),
        });
    }

    return containers;
}

fn collectReposInContainer(allocator: Allocator, container_name: []const u8) !std.ArrayList([]const u8) {
    const result = try runDocker(allocator, &.{
        "docker",
        "exec",
        container_name,
        "sh",
        "-lc",
        "for p in /repos/*; do [ -d \"$p\" ] && basename \"$p\"; done",
    });
    if (!isExitedZero(result.term)) {
        std.debug.print("Error: failed to list repos in container: {s}\n", .{container_name});
        printCommandOutput(result);
        return error.ListReposFailed;
    }

    var repos = std.ArrayList([]const u8).empty;
    var lines = std.mem.splitScalar(u8, result.stdout, '\n');
    while (lines.next()) |line_raw| {
        const line = std.mem.trim(u8, line_raw, " \t\r\n");
        if (line.len == 0) continue;
        try repos.append(allocator, try allocator.dupe(u8, line));
    }

    return repos;
}

fn selectContainerByPrompt(allocator: Allocator, containers: []const Container) !usize {
    for (containers, 0..) |container, idx| {
        std.debug.print("  [{d}] {s} ({s})\n", .{ idx + 1, container.name, container.status });
    }
    const prompt = try std.fmt.allocPrint(
        allocator,
        "Select container (1-{d}, default 1): ",
        .{containers.len},
    );
    const input = try promptLine(allocator, prompt);
    const trimmed = std.mem.trim(u8, input, " \t\r\n");

    if (trimmed.len == 0) return 0;
    const parsed = std.fmt.parseInt(usize, trimmed, 10) catch return 0;
    if (parsed < 1 or parsed > containers.len) return 0;
    return parsed - 1;
}

fn findContainerBySelector(containers: []const Container, selector: []const u8) !usize {
    var exact_idx: ?usize = null;
    for (containers, 0..) |container, idx| {
        if (std.mem.eql(u8, container.name, selector) or std.mem.eql(u8, container.id, selector)) {
            if (exact_idx != null) return error.AmbiguousSelector;
            exact_idx = idx;
        }
    }
    if (exact_idx) |idx| return idx;

    var prefix_idx: ?usize = null;
    for (containers, 0..) |container, idx| {
        if (std.mem.startsWith(u8, container.id, selector)) {
            if (prefix_idx != null) return error.AmbiguousSelector;
            prefix_idx = idx;
        }
    }
    if (prefix_idx) |idx| return idx;

    return error.ContainerNotFound;
}

fn findRepoByName(repos: []const []const u8, repo_name: []const u8) !usize {
    var found_idx: ?usize = null;
    for (repos, 0..) |repo, idx| {
        if (std.mem.eql(u8, repo, repo_name)) {
            if (found_idx != null) return error.AmbiguousRepo;
            found_idx = idx;
        }
    }
    return found_idx orelse error.RepoNotFound;
}

fn selectRepoByPrompt(allocator: Allocator, repos: []const []const u8) !usize {
    for (repos, 0..) |repo, idx| {
        std.debug.print("  [{d}] {s}\n", .{ idx + 1, repo });
    }
    const prompt = try std.fmt.allocPrint(
        allocator,
        "Select repo (1-{d}, default 1): ",
        .{repos.len},
    );
    const input = try promptLine(allocator, prompt);
    const trimmed = std.mem.trim(u8, input, " \t\r\n");

    if (trimmed.len == 0) return 0;
    const parsed = std.fmt.parseInt(usize, trimmed, 10) catch return 0;
    if (parsed < 1 or parsed > repos.len) return 0;
    return parsed - 1;
}

fn selectRepoByArrow(allocator: Allocator, repos: []const []const u8) !usize {
    if (builtin.os.tag == .windows) return error.UnsupportedOperatingSystem;
    if (!std.fs.File.stdin().isTty()) return error.NotATerminal;
    if (repos.len == 0) return error.EmptyRepoList;

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
            std.debug.print("\x1b[{d}A", .{repos.len + 1});
        } else {
            rendered_once = true;
        }

        std.debug.print("\x1b[2K\rUse Up/Down arrows, Enter to confirm\n", .{});
        for (repos, 0..) |repo, idx| {
            const prefix = if (idx == selected) "> " else "  ";
            std.debug.print("\x1b[2K\r{s}[{d}] {s}\n", .{ prefix, idx + 1, repo });
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
                if (selected + 1 < repos.len) selected += 1;
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
                    if (selected + 1 < repos.len) selected += 1;
                }
            },
            else => {},
        }
    }

    return selected;
}

fn selectRepoIndex(allocator: Allocator, repos: []const []const u8) !usize {
    return selectRepoByArrow(allocator, repos) catch {
        return try selectRepoByPrompt(allocator, repos);
    };
}

fn spawnAnalyzeInRepo(
    allocator: Allocator,
    container_name: []const u8,
    repo_name: []const u8,
    analyze_args: []const []const u8,
    attach_tty: bool,
    inherit_stdin: bool,
) !std.process.Child {
    const workdir = try std.fmt.allocPrint(allocator, "/repos/{s}", .{repo_name});

    var docker_args = std.ArrayList([]const u8).empty;
    try docker_args.appendSlice(allocator, &.{ "docker", "exec" });

    if (attach_tty) {
        try docker_args.append(allocator, "-t");
    }
    if (inherit_stdin) {
        try docker_args.append(allocator, "-i");
    }

    try docker_args.appendSlice(allocator, &.{ "-w", workdir, container_name, "gitnexus", "analyze" });
    try docker_args.appendSlice(allocator, analyze_args);

    var child = std.process.Child.init(docker_args.items, allocator);
    child.stdin_behavior = if (inherit_stdin) .Inherit else .Ignore;
    child.stdout_behavior = .Inherit;
    child.stderr_behavior = .Inherit;
    try child.spawn();
    return child;
}

fn runAnalyzeInRepo(
    allocator: Allocator,
    container_name: []const u8,
    repo_name: []const u8,
    analyze_args: []const []const u8,
) !u8 {
    const use_tty = std.fs.File.stdin().isTty() and std.fs.File.stdout().isTty();
    var child = try spawnAnalyzeInRepo(
        allocator,
        container_name,
        repo_name,
        analyze_args,
        use_tty,
        true,
    );
    return exitCodeFromTerm(try child.wait());
}

fn runAnalyzeAllRepos(
    allocator: Allocator,
    container_name: []const u8,
    repos: []const []const u8,
    analyze_args: []const []const u8,
    jobs: usize,
) !u8 {
    const use_tty = std.fs.File.stdout().isTty();
    std.debug.print("Running analyze for all repos in container: {s} (jobs={d})\n", .{
        container_name,
        jobs,
    });

    var active = std.ArrayList(ActiveAnalyze).empty;
    var next_repo_index: usize = 0;
    var completed_count: usize = 0;
    var failed_count: usize = 0;
    std.debug.print("[progress] 0/{d} completed, running 0, failed 0\n", .{repos.len});

    while (next_repo_index < repos.len or active.items.len != 0) {
        while (next_repo_index < repos.len and active.items.len < jobs) {
            const repo = repos[next_repo_index];
            std.debug.print("\n[{s}] starting gitnexus analyze ({d}/{d})\n", .{
                repo,
                next_repo_index + 1,
                repos.len,
            });

            const child = try spawnAnalyzeInRepo(
                allocator,
                container_name,
                repo,
                analyze_args,
                use_tty,
                false,
            );
            try active.append(allocator, .{
                .repo = repo,
                .child = child,
            });
            next_repo_index += 1;
        }

        if (active.items.len == 0) continue;

        var done = active.orderedRemove(0);
        const code = exitCodeFromTerm(try done.child.wait());
        completed_count += 1;
        if (code != 0) {
            failed_count += 1;
            std.debug.print("[{s}] failed with exit code {d}\n", .{ done.repo, code });
        } else {
            std.debug.print("[{s}] done\n", .{done.repo});
        }
        std.debug.print(
            "[progress] {d}/{d} completed, running {d}, failed {d}\n",
            .{ completed_count, repos.len, active.items.len, failed_count },
        );
    }

    if (failed_count != 0) {
        std.debug.print(
            "\nError: analyze failed in {d}/{d} repos.\n",
            .{ failed_count, repos.len },
        );
        return 1;
    }

    std.debug.print("\nAnalyze succeeded for all {d} repos.\n", .{repos.len});
    return 0;
}

fn run(allocator: Allocator) !u8 {
    const args = try std.process.argsAlloc(allocator);
    defer std.process.argsFree(allocator, args);

    var container_selector: ?[]const u8 = null;
    var repo_selector: ?[]const u8 = null;
    var analyze_all = false;
    var jobs: usize = 1;
    var passthrough_start: usize = args.len;

    var i: usize = 1;
    while (i < args.len) : (i += 1) {
        const arg = args[i];
        if (std.mem.eql(u8, arg, "-h") or std.mem.eql(u8, arg, "--help")) {
            printUsage(args[0]);
            return 0;
        }
        if (std.mem.eql(u8, arg, "-c") or std.mem.eql(u8, arg, "--container")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                printUsage(args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: container selector must not be empty\n", .{});
                return 1;
            }
            container_selector = value;
            continue;
        }
        if (std.mem.eql(u8, arg, "-r") or std.mem.eql(u8, arg, "--repo")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                printUsage(args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: repo selector must not be empty\n", .{});
                return 1;
            }
            repo_selector = value;
            continue;
        }
        if (std.mem.eql(u8, arg, "-a") or std.mem.eql(u8, arg, "--all")) {
            analyze_all = true;
            continue;
        }
        if (std.mem.eql(u8, arg, "-j") or std.mem.eql(u8, arg, "--jobs")) {
            i += 1;
            if (i >= args.len) {
                std.debug.print("Error: missing value for {s}\n", .{arg});
                printUsage(args[0]);
                return 1;
            }
            const value = std.mem.trim(u8, args[i], " \t\r\n");
            if (value.len == 0) {
                std.debug.print("Error: jobs must not be empty\n", .{});
                return 1;
            }
            const parsed = std.fmt.parseInt(usize, value, 10) catch {
                std.debug.print("Error: invalid jobs value: {s}\n", .{value});
                return 1;
            };
            if (parsed < 1) {
                std.debug.print("Error: jobs must be >= 1\n", .{});
                return 1;
            }
            jobs = parsed;
            continue;
        }
        if (std.mem.eql(u8, arg, "--")) {
            passthrough_start = i + 1;
            break;
        }

        passthrough_start = i;
        break;
    }

    const analyze_args = args[passthrough_start..];

    if (analyze_all and repo_selector != null) {
        std.debug.print("Error: --repo and --all cannot be used together.\n", .{});
        printUsage(args[0]);
        return 1;
    }
    if (!analyze_all and jobs != 1) {
        std.debug.print("Error: --jobs requires --all.\n", .{});
        printUsage(args[0]);
        return 1;
    }

    const containers = collectRunningContainers(allocator) catch |err| switch (err) {
        error.DockerPsFailed => return 1,
        else => return err,
    };
    if (containers.items.len == 0) {
        std.debug.print("Error: no running managed container found (expected names like {s} or {s}-*)\n", .{
            container_name_prefix,
            container_name_prefix,
        });
        return 1;
    }

    const target_index = if (container_selector) |selector|
        findContainerBySelector(containers.items, selector) catch |err| switch (err) {
            error.ContainerNotFound => {
                std.debug.print("Error: target container not found: {s}\n", .{selector});
                std.debug.print("Available running containers:\n", .{});
                for (containers.items) |container| {
                    std.debug.print("  {s} ({s})\n", .{ container.name, container.id });
                }
                return 1;
            },
            error.AmbiguousSelector => {
                std.debug.print("Error: ambiguous container selector: {s}\n", .{selector});
                std.debug.print("Please specify full name/id. Available:\n", .{});
                for (containers.items) |container| {
                    std.debug.print("  {s} ({s})\n", .{ container.name, container.id });
                }
                return 1;
            },
            else => return err,
        }
    else if (containers.items.len == 1)
        0
    else blk: {
        std.debug.print("Multiple running containers found:\n", .{});
        break :blk try selectContainerByPrompt(allocator, containers.items);
    };

    const target = containers.items[target_index];
    std.debug.print("Using container: {s}\n", .{target.name});

    const repos = collectReposInContainer(allocator, target.name) catch |err| switch (err) {
        error.ListReposFailed => return 1,
        else => return err,
    };
    if (repos.items.len == 0) {
        std.debug.print("Error: no repo found under /repos in container: {s}\n", .{target.name});
        return 1;
    }

    if (analyze_all) return try runAnalyzeAllRepos(
        allocator,
        target.name,
        repos.items,
        analyze_args,
        jobs,
    );

    const target_repo_index = if (repo_selector) |repo_name|
        findRepoByName(repos.items, repo_name) catch |err| switch (err) {
            error.RepoNotFound => {
                std.debug.print("Error: repo not found in container: {s}\n", .{repo_name});
                std.debug.print("Available repos:\n", .{});
                for (repos.items) |repo| {
                    std.debug.print("  {s}\n", .{repo});
                }
                return 1;
            },
            else => return err,
        }
    else if (repos.items.len == 1)
        0
    else blk: {
        std.debug.print("Multiple repos found in container:\n", .{});
        break :blk try selectRepoIndex(allocator, repos.items);
    };

    const target_repo = repos.items[target_repo_index];
    std.debug.print("Using repo: {s}\n", .{target_repo});
    return try runAnalyzeInRepo(allocator, target.name, target_repo, analyze_args);
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

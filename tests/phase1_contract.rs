use std::env;
use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

struct TempDirGuard {
    path: PathBuf,
}

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let mut path = env::temp_dir();
        path.push(format!(
            "gitnexus-rs-{prefix}-{}-{}",
            std::process::id(),
            unique_suffix()
        ));
        fs::create_dir_all(&path).expect("failed to create temp directory");
        Self { path }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[derive(Debug)]
struct HttpResponse {
    status: u16,
    body: String,
}

#[test]
fn augment_short_input_is_noop_with_empty_stderr() {
    let sandbox = TempDirGuard::new("augment-noop");
    let home = sandbox.path().join("home");
    let work = sandbox.path().join("work");
    fs::create_dir_all(&home).expect("failed to create HOME dir");
    fs::create_dir_all(&work).expect("failed to create work dir");

    let output = run_gitnexus(
        ["augment", "ab"],
        &home,
        Some(&work),
        Stdio::piped(),
        Stdio::piped(),
    );

    assert!(
        output.status.success(),
        "augment short-input should succeed, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "augment short-input should not write stderr, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn augment_query_failure_is_silent_and_successful() {
    let sandbox = TempDirGuard::new("augment-failure");
    let home = sandbox.path().join("home");
    let work = sandbox.path().join("work");
    fs::create_dir_all(&home).expect("failed to create HOME dir");
    fs::create_dir_all(&work).expect("failed to create work dir");

    let output = run_gitnexus(
        ["augment", "phase1_contract_missing_repo"],
        &home,
        Some(&work),
        Stdio::piped(),
        Stdio::piped(),
    );

    assert!(
        output.status.success(),
        "augment failure-path should still succeed, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).trim().is_empty(),
        "augment failure-path should stay silent on stderr, got: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn augment_success_writes_hook_output_to_stderr() {
    let sandbox = TempDirGuard::new("augment-success");
    let home = sandbox.path().join("home");
    let repo = sandbox.path().join("repo");
    fs::create_dir_all(&home).expect("failed to create HOME dir");

    init_test_repo(&repo);
    analyze_repo(&home, &repo);

    let output = run_gitnexus(
        ["augment", "phase_one_contract_target"],
        &home,
        Some(&repo),
        Stdio::piped(),
        Stdio::piped(),
    );

    assert!(
        output.status.success(),
        "augment success-path should succeed, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("[GitNexus]"),
        "augment success-path should emit hook text to stderr, got: {stderr}"
    );
    assert!(
        stderr.contains("phase_one_contract_target"),
        "augment output should include matched symbol, got: {stderr}"
    );
}

#[test]
fn eval_server_http_endpoints_follow_contract() {
    let sandbox = TempDirGuard::new("eval-server");
    let home = sandbox.path().join("home");
    let repo = sandbox.path().join("repo");
    fs::create_dir_all(&home).expect("failed to create HOME dir");

    init_test_repo(&repo);
    analyze_repo(&home, &repo);

    let port = reserve_port();
    let mut child = spawn_eval_server(&home, &repo, port, 30);
    wait_for_server_ready(&mut child, port, Duration::from_secs(10));

    let health = http_request("GET", port, "/health", None);
    assert_eq!(
        health.status, 200,
        "GET /health expected 200, body: {}",
        health.body
    );
    assert!(
        health.body.contains("\"status\":\"ok\""),
        "/health should report ok, body: {}",
        health.body
    );
    assert!(
        health.body.contains("\"repos\""),
        "/health should include repos, body: {}",
        health.body
    );

    let query = http_request(
        "POST",
        port,
        "/tool/query",
        Some(r#"{"query":"phase1_contract_no_hits_keyword","limit":1}"#),
    );
    assert_eq!(
        query.status, 200,
        "POST /tool/query expected 200, body: {}",
        query.body
    );
    assert!(
        query.body.contains(
            "No matching execution flows found. Try a different search term or use grep."
        ),
        "/tool/query no-hit text mismatch, body: {}",
        query.body
    );
    assert!(
        query
            .body
            .contains("Next: Pick a symbol above and run gitnexus context"),
        "/tool/query should include next-step hint, body: {}",
        query.body
    );

    let shutdown = http_request("POST", port, "/shutdown", Some("{}"));
    assert_eq!(
        shutdown.status, 200,
        "POST /shutdown expected 200, body: {}",
        shutdown.body
    );
    assert!(
        shutdown.body.contains("shutting_down"),
        "/shutdown should return shutting_down, body: {}",
        shutdown.body
    );

    let status = wait_for_child_exit(&mut child, Duration::from_secs(8));
    assert!(
        status.success(),
        "eval-server should exit cleanly after /shutdown, status: {status}"
    );
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos()
}

fn gitnexus_bin() -> PathBuf {
    if let Ok(path) = env::var("CARGO_BIN_EXE_gitnexus-rs") {
        return PathBuf::from(path);
    }
    if let Ok(path) = env::var("CARGO_BIN_EXE_gitnexus_rs") {
        return PathBuf::from(path);
    }
    panic!("CARGO_BIN_EXE_gitnexus-rs/CARGO_BIN_EXE_gitnexus_rs is not set");
}

fn init_test_repo(repo: &Path) {
    fs::create_dir_all(repo.join("src")).expect("failed to create src directory");
    fs::write(
        repo.join("src/lib.rs"),
        "pub fn phase_one_contract_target() -> &'static str { \"ok\" }\n",
    )
    .expect("failed to write test source");

    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(repo)
        .status()
        .expect("failed to run git init");
    assert!(status.success(), "git init failed with status: {status}");
}

fn analyze_repo(home: &Path, repo: &Path) {
    let output = run_gitnexus(
        [
            "analyze",
            repo.to_str().expect("repo path utf-8"),
            "--force",
        ],
        home,
        Some(repo),
        Stdio::piped(),
        Stdio::piped(),
    );

    assert!(
        output.status.success(),
        "analyze failed, stdout: {}, stderr: {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn run_gitnexus<const N: usize>(
    args: [&str; N],
    home: &Path,
    cwd: Option<&Path>,
    stdout: Stdio,
    stderr: Stdio,
) -> Output {
    let mut cmd = Command::new(gitnexus_bin());
    cmd.args(args)
        .env("HOME", home)
        .stdout(stdout)
        .stderr(stderr);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }

    cmd.output().expect("failed to run gitnexus command")
}

fn spawn_eval_server(home: &Path, repo: &Path, port: u16, idle_timeout: u32) -> Child {
    Command::new(gitnexus_bin())
        .args([
            "eval-server",
            "--port",
            &port.to_string(),
            "--idle-timeout",
            &idle_timeout.to_string(),
        ])
        .env("HOME", home)
        .current_dir(repo)
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .expect("failed to spawn eval-server")
}

fn reserve_port() -> u16 {
    let listener =
        TcpListener::bind(("127.0.0.1", 0)).expect("failed to reserve local ephemeral port");
    listener.local_addr().expect("local addr").port()
}

fn wait_for_server_ready(child: &mut Child, port: u16, timeout: Duration) {
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .expect("failed to poll eval-server process")
        {
            panic!("eval-server exited before readiness, status: {status}");
        }

        if TcpStream::connect(("127.0.0.1", port)).is_ok() {
            return;
        }

        if start.elapsed() > timeout {
            let _ = child.kill();
            let _ = child.wait();
            panic!("timed out waiting for eval-server readiness on port {port}");
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn wait_for_child_exit(child: &mut Child, timeout: Duration) -> ExitStatus {
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("failed to poll child process") {
            return status;
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            let status = child.wait().expect("failed to wait eval-server after kill");
            panic!("child did not exit within timeout, killed with status: {status}");
        }
        thread::sleep(Duration::from_millis(50));
    }
}

fn http_request(method: &str, port: u16, path: &str, body: Option<&str>) -> HttpResponse {
    let body = body.unwrap_or("");
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    );

    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).expect("failed to connect to eval-server");
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .expect("failed to set read timeout");
    stream
        .write_all(request.as_bytes())
        .expect("failed to send HTTP request");
    stream.flush().expect("failed to flush HTTP request");

    let mut raw = String::new();
    stream
        .read_to_string(&mut raw)
        .expect("failed to read HTTP response");

    let (head, body) = raw
        .split_once("\r\n\r\n")
        .expect("HTTP response missing header/body separator");
    let status_line = head.lines().next().unwrap_or_default();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|v| v.parse::<u16>().ok())
        .expect("failed to parse HTTP status code");

    HttpResponse {
        status,
        body: body.to_string(),
    }
}

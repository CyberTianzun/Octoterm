//! 设置页写入 desktop 自身的 hook 必须通过真实 shell 执行,且不能初始化 GUI。

use std::io::{Read, Write};
use std::net::TcpListener;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn run_hook(mut command: Command, managed: bool) -> std::process::Output {
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if managed {
        command
            .env("OCTOTERM_SESSION_ID", "test-session")
            .env("OCTOTERM_HOOK_TOKEN", "test-token");
    } else {
        command
            .env_remove("OCTOTERM_SESSION_ID")
            .env_remove("OCTOTERM_HOOK_TOKEN");
    }
    let mut child = command.spawn().unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(r#"{"tool_name":"shell","message":"中文"}"#.as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            let _ = child.wait();
            panic!("desktop hook did not exit; it may have entered GUI startup");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
    output
}

#[test]
fn desktop_hook_is_silent_without_managed_environment() {
    let dir = tempfile::tempdir().unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_octoterm-desktop"));
    command.args(["hook", "http://127.0.0.1:1/hook/codex/stop"]);
    command
        .env("APPDATA", dir.path())
        .env("XDG_CONFIG_HOME", dir.path());
    let output = run_hook(command, false);
    assert!(output.stdout.is_empty());
    assert_eq!(
        std::fs::read_dir(dir.path()).unwrap().count(),
        0,
        "hook initialized desktop config/logs"
    );
}

#[test]
fn installed_codex_command_forwards_stdin_and_reply_through_desktop() {
    installed_command_forwards_stdin_and_reply_through_desktop("codex");
}

#[cfg(windows)]
#[test]
fn installed_antigravity_command_forwards_stdin_and_reply_through_cmd() {
    installed_command_forwards_stdin_and_reply_through_desktop("antigravity");
}

#[cfg(windows)]
#[test]
fn installed_antigravity_command_returns_neutral_json_without_server_or_environment() {
    let text = octoterm_server::agent::hook_cli::command_for_executable(
        "antigravity", 1, "Stop", std::path::Path::new(env!("CARGO_BIN_EXE_octoterm-desktop")));
    for managed in [false, true] {
        let mut command = Command::new("cmd.exe");
        command.args(["/C", &text]);
        assert_eq!(run_hook(command, managed).stdout, b"{}");
    }
}

fn installed_command_forwards_stdin_and_reply_through_desktop(agent: &str) {
    use octoterm_server::agent::{edit::InstallCtx, find};
    let home = tempfile::tempdir().unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let plan = find(agent)
        .unwrap()
        .plan_install(&InstallCtx {
            home: home.path().into(),
            port,
            include_blocking: true,
        })
        .unwrap();
    let mut doc = serde_json::json!({});
    for edit in plan {
        octoterm_server::agent::edit::apply_to_json(&mut doc, &edit.op).unwrap();
    }
    let installed = if agent == "antigravity" {
        &doc["octoterm"]["PreInvocation"][0]["command"]
    } else {
        &doc["hooks"]["PreToolUse"][0]["hooks"][0]["command"]
    }
        .as_str()
        .unwrap();
    let exe = std::env::current_exe()
        .unwrap()
        .to_string_lossy()
        .to_string();
    // 安装路径允许空格、中文和 PowerShell 元字符,不能只测无空格的开发目录。
    let executable_dir = home.path().join(if cfg!(windows) {
        "Octoterm 中文 $cash 'quoted' %PATH% & (safe)!"
    } else {
        "Octoterm 中文 spaces"
    });
    std::fs::create_dir(&executable_dir).unwrap();
    let desktop_path = executable_dir.join(if cfg!(windows) {
        "octoterm-desktop.exe"
    } else {
        "octoterm-desktop"
    });
    std::fs::copy(env!("CARGO_BIN_EXE_octoterm-desktop"), &desktop_path).unwrap();
    let desktop = desktop_path.to_str().unwrap();
    let command_text = if agent == "antigravity" {
        // The path is inside an encoded PowerShell command, so generate the same
        // installed command using the copied desktop executable's path.
        octoterm_server::agent::hook_cli::command_for_executable(agent, port, "PreInvocation", &desktop_path)
    } else if cfg!(windows) {
        installed.replace(&exe.replace('\'', "''"), &desktop.replace('\'', "''"))
    } else {
        installed.replace(&exe, desktop)
    };
    let endpoint = if agent == "antigravity" { "antigravity/pre-invocation" } else { "codex/pre-tool-use" };
    let server = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    assert!(Instant::now() < deadline, "installed hook never connected");
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => panic!("{e}"),
            }
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut request = Vec::new();
        loop {
            let mut buf = [0; 4096];
            let n = stream.read(&mut buf).unwrap();
            assert_ne!(n, 0);
            request.extend_from_slice(&buf[..n]);
            if let Some(end) = request.windows(4).position(|w| w == b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&request[..end]);
                let length: usize = head
                    .lines()
                    .find_map(|line| line.strip_prefix("Content-Length: "))
                    .unwrap()
                    .parse()
                    .unwrap();
                if request.len() >= end + 4 + length {
                    break;
                }
            }
        }
        let request = String::from_utf8(request).unwrap();
        assert!(request.starts_with(&format!("POST /hook/{endpoint} HTTP/1.1\r\n")));
        assert!(request.contains("Authorization: Bearer test-token\r\n"));
        assert!(request.contains("X-Octoterm-Session: test-session\r\n"));
        assert!(request.ends_with(r#"{"tool_name":"shell","message":"中文"}"#));
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            .unwrap();
    });
    let mut command = if cfg!(windows) && agent == "antigravity" {
        let mut c = Command::new("cmd.exe");
        c.arg("/C");
        c
    } else if cfg!(windows) {
        let mut c = Command::new("powershell.exe");
        c.args(["-NoLogo", "-NoProfile", "-NonInteractive", "-Command"]);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c");
        c
    };
    command.arg(command_text);
    let output = run_hook(command, true);
    server.join().unwrap();
    assert_eq!(output.stdout, b"{}");
}

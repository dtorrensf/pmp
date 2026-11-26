// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::fs;
use std::process::Command;

#[test]
fn main_rs_is_a_thin_entrypoint() {
    let source = fs::read_to_string("src/main.rs").expect("src/main.rs should exist");
    let lines: Vec<&str> = source.lines().collect();
    assert!(
        lines.len() <= 60,
        "main.rs must stay a thin entrypoint, got {} lines",
        lines.len()
    );
    assert!(
        !source.contains("ratatui::"),
        "main.rs must not import ratatui directly"
    );
    assert!(
        !source.contains("Block"),
        "main.rs must not contain widget logic"
    );
    assert!(
        !source.contains("Paragraph"),
        "main.rs must not contain widget logic"
    );
    assert!(
        source.contains("pmp::app::App"),
        "main.rs must construct App through the library"
    );
    assert!(
        source.contains("pmp::tui::run"),
        "main.rs must delegate to pmp::tui::run"
    );
    assert!(
        source.contains("#[tokio::main"),
        "main.rs must use tokio::main"
    );
    assert!(
        source.contains("std::env::args"),
        "main.rs must read command-line args"
    );
    assert!(
        source.contains("mcp"),
        "main.rs must branch on the mcp subcommand"
    );
    assert!(
        source.contains("pmp::mcp::serve"),
        "main.rs must delegate mcp path to pmp::mcp::serve"
    );
}

#[test]
fn main_rs_injects_the_resolved_configuration() {
    let source = fs::read_to_string("src/main.rs").expect("src/main.rs should exist");
    assert!(
        source.contains("Paths::discover"),
        "main.rs must resolve the XDG paths once, in the composition root"
    );
    assert!(
        source.contains("Config::load"),
        "main.rs must resolve the configuration once, in the composition root"
    );
    assert!(
        source.contains("config.database.busy"),
        "main.rs must inject the database lock policy"
    );
    assert!(
        source.contains("config.mcp.timeout"),
        "main.rs must inject the MCP timeout"
    );
    assert!(
        source.contains("config.appearance"),
        "main.rs must inject the appearance settings"
    );
    assert!(
        source.contains("logging::init"),
        "main.rs must install the file logger for the TUI and the MCP server"
    );
    assert!(
        source.contains("pmp::doctor::run"),
        "main.rs must delegate the doctor subcommand"
    );
}

#[test]
fn paths_module_owns_the_xdg_layout() {
    let source = fs::read_to_string("src/paths.rs").expect("src/paths.rs should exist");
    assert!(
        source.contains("dirs::data_dir"),
        "paths.rs must resolve the XDG data directory"
    );
    assert!(source.contains("pmp.db"), "paths.rs must own pmp.db");
    assert!(
        source.contains("logs"),
        "paths.rs must own the logs directory"
    );
}

#[test]
fn binary_mcp_arg_responds_to_initialize_then_exits_cleanly() {
    let temp_dir = tempfile::tempdir().unwrap();
    let bin_path = env!("CARGO_BIN_EXE_pmp");
    let mut child = Command::new(bin_path)
        .arg("mcp")
        .env("XDG_DATA_HOME", temp_dir.path())
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("pmp mcp binary should spawn");

    let stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");

    let init_request = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "1.0" }
        }
    });

    let mut writer = stdin;
    std::io::Write::write_all(&mut writer, init_request.to_string().as_bytes()).unwrap();
    std::io::Write::write_all(&mut writer, b"\n").unwrap();
    drop(writer);

    let mut reader = std::io::BufReader::new(stdout);
    let mut line = String::new();
    std::io::BufRead::read_line(&mut reader, &mut line).unwrap();

    let response: serde_json::Value = serde_json::from_str(&line).expect("response should be json");
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert!(
        response["result"]["capabilities"]["tools"].is_object(),
        "initialize result should advertise tools capability: {}",
        response
    );
    assert!(
        response["result"]["capabilities"]["prompts"].is_object(),
        "initialize result should advertise prompts capability: {}",
        response
    );
    assert_eq!(
        response["result"]["capabilities"]["prompts"]["listChanged"],
        false
    );

    let status = child.wait().expect("child should exit");
    assert!(status.success(), "pmp mcp should exit cleanly");
    assert!(
        temp_dir.path().join("pmp/logs/pmp.log").is_file(),
        "pmp mcp must create its log file without touching stdout"
    );
}

#[test]
fn binary_doctor_reports_the_environment() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_pmp"))
        .arg("doctor")
        .env("XDG_DATA_HOME", data.path())
        .env("XDG_CONFIG_HOME", config.path())
        .output()
        .expect("pmp doctor should spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        output.status.success(),
        "pmp doctor should succeed:\n{stdout}"
    );
    assert!(stdout.starts_with("pmp doctor "), "{stdout}");
    assert!(stdout.contains("version: "), "{stdout}");
    assert!(stdout.contains("pmp.db: "), "{stdout}");
    assert!(stdout.contains("summary: "), "{stdout}");
}

#[test]
fn binary_doctor_exits_nonzero_when_a_check_fails() {
    let data = tempfile::tempdir().unwrap();
    let config = tempfile::tempdir().unwrap();
    // A regular file where the data directory belongs makes every probe fail.
    fs::write(data.path().join("pmp"), b"not a directory").unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_pmp"))
        .arg("doctor")
        .env("XDG_DATA_HOME", data.path())
        .env("XDG_CONFIG_HOME", config.path())
        .output()
        .expect("pmp doctor should spawn");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !output.status.success(),
        "pmp doctor should fail:\n{stdout}"
    );
    assert!(stdout.contains("fail  data directory"), "{stdout}");
}

#[test]
fn binary_version_arg_prints_version_and_exits_cleanly() {
    let expected = format!("{}\n", env!("CARGO_PKG_VERSION"));
    for flag in ["--version", "-v"] {
        let output = Command::new(env!("CARGO_BIN_EXE_pmp"))
            .arg(flag)
            .output()
            .expect("pmp version should spawn");
        assert!(output.status.success(), "pmp {flag} should exit cleanly");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            expected,
            "pmp {flag} should print the version"
        );
    }
}

#[test]
fn binary_help_arg_prints_usage_and_exits_cleanly() {
    for flag in ["--help", "-h"] {
        let output = Command::new(env!("CARGO_BIN_EXE_pmp"))
            .arg(flag)
            .output()
            .expect("pmp help should spawn");
        assert!(output.status.success(), "pmp {flag} should exit cleanly");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("pmp - Project Management Program"),
            "pmp {flag} should print the program title"
        );
        assert!(stdout.contains("Usage:"), "pmp {flag} should print usage");
        assert!(
            stdout.contains("Options:"),
            "pmp {flag} should print options"
        );
        assert!(
            stdout.contains("Commands:"),
            "pmp {flag} should print commands"
        );
    }
}

#[test]
fn binary_invalid_arg_prints_help_and_exits_with_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_pmp"))
        .arg("invalid-arg")
        .output()
        .expect("pmp invalid-arg should spawn");
    assert!(
        !output.status.success(),
        "pmp invalid-arg should exit with an error"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("error:"), "stderr should contain error");
    assert!(
        stderr.contains("Usage:"),
        "stderr should contain usage help"
    );
}

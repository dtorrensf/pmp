// Copyright (C) 2026 Diego Torréns Farias
// SPDX-License-Identifier: AGPL-3.0-or-later
use std::fs;
use std::process::Command;

#[test]
fn main_rs_is_a_thin_entrypoint() {
    let source = fs::read_to_string("src/main.rs").expect("src/main.rs should exist");
    let lines: Vec<&str> = source.lines().collect();
    assert!(
        lines.len() <= 35,
        "main.rs must be at most 35 lines, got {}",
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
fn main_rs_opens_xdg_database_path() {
    let source = fs::read_to_string("src/main.rs").expect("src/main.rs should exist");
    assert!(
        source.contains("dirs::data_dir"),
        "main.rs must resolve the XDG data directory"
    );
    assert!(source.contains("pmp.db"), "main.rs must open pmp.db");
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

    let status = child.wait().expect("child should exit");
    assert!(status.success(), "pmp mcp should exit cleanly");
}

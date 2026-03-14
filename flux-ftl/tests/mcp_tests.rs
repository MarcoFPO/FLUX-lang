use std::io::Write;
use std::process::{Command, Stdio};

fn mcp_cmd() -> Command {
    Command::new(env!("CARGO_BIN_EXE_flux-mcp"))
}

fn send_request(input: &str) -> (String, String) {
    let mut child = mcp_cmd()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn flux-mcp");

    {
        let stdin = child.stdin.as_mut().expect("failed to open stdin");
        stdin.write_all(input.as_bytes()).expect("failed to write");
    }

    let output = child.wait_with_output().expect("failed to wait");
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (stdout, stderr)
}

fn send_jsonrpc(method: &str, params: Option<serde_json::Value>) -> serde_json::Value {
    let request = if let Some(p) = params {
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": p})
    } else {
        serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": method})
    };

    let input = format!("{}\n", request);
    let (stdout, _) = send_request(&input);

    let first_line = stdout.lines().next().expect("no output from MCP server");
    serde_json::from_str(first_line).expect("invalid JSON response")
}

#[test]
fn mcp_initialize() {
    let resp = send_jsonrpc("initialize", None);

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert!(resp["error"].is_null());

    let result = &resp["result"];
    assert_eq!(result["protocolVersion"], "2024-11-05");
    assert_eq!(result["serverInfo"]["name"], "flux-ftl");
    assert_eq!(result["serverInfo"]["version"], "1.0.0");
    assert!(result["capabilities"]["tools"].is_object());
}

#[test]
fn mcp_tools_list() {
    let resp = send_jsonrpc("tools/list", None);

    let tools = resp["result"]["tools"].as_array().expect("tools should be array");
    assert_eq!(tools.len(), 6);

    let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"flux_check"));
    assert!(names.contains(&"flux_compile"));
    assert!(names.contains(&"flux_build"));
    assert!(names.contains(&"flux_ir"));
    assert!(names.contains(&"flux_evolve"));
    assert!(names.contains(&"flux_prove"));

    for tool in tools {
        assert!(tool["description"].is_string());
        assert!(tool["inputSchema"].is_object());
        assert_eq!(tool["inputSchema"]["type"], "object");
        let required = tool["inputSchema"]["required"].as_array().unwrap();
        assert!(required.iter().any(|r| r == "ftl_source"));
    }
}

#[test]
fn mcp_flux_check_valid() {
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").expect("read testdata");

    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "flux_check",
        "arguments": { "ftl_source": ftl }
    })));

    assert!(resp["error"].is_null());
    let content = &resp["result"]["content"];
    assert!(content.is_array());
    let text = content[0]["text"].as_str().expect("text content");
    let check_result: serde_json::Value = serde_json::from_str(text).expect("valid JSON in text");
    assert_eq!(check_result["status"], "OK");
}

#[test]
fn mcp_flux_check_invalid() {
    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "flux_check",
        "arguments": { "ftl_source": "this is not valid FTL" }
    })));

    assert!(resp["error"].is_null());
    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text content");
    let check_result: serde_json::Value = serde_json::from_str(text).expect("valid JSON in text");
    assert_eq!(check_result["status"], "PARSE_ERROR");
}

#[test]
fn mcp_flux_compile_valid() {
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").expect("read testdata");

    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "flux_compile",
        "arguments": { "ftl_source": ftl }
    })));

    assert!(resp["error"].is_null());
    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text content");
    let compile_result: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
    assert!(compile_result["entry_hash"].is_string());
    assert!(compile_result["total_nodes"].is_number());
}

#[test]
fn mcp_flux_prove_valid() {
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").expect("read testdata");

    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "flux_prove",
        "arguments": { "ftl_source": ftl }
    })));

    assert!(resp["error"].is_null());
    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text content");
    let proof_results: serde_json::Value = serde_json::from_str(text).expect("valid JSON");
    assert!(proof_results.is_array());

    let results = proof_results.as_array().unwrap();
    assert!(!results.is_empty());
    for r in results {
        assert!(r["contract_id"].is_string());
        assert!(r["status"].is_string());
    }
}

#[test]
fn mcp_flux_ir_valid() {
    let ftl = std::fs::read_to_string("testdata/hello_world.ftl").expect("read testdata");

    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "flux_ir",
        "arguments": { "ftl_source": ftl }
    })));

    assert!(resp["error"].is_null());
    let content = &resp["result"]["content"];
    let text = content[0]["text"].as_str().expect("text content");
    assert!(text.contains("define"), "IR should contain 'define'");
}

#[test]
fn mcp_unknown_tool() {
    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "nonexistent_tool",
        "arguments": {}
    })));

    assert!(resp["error"].is_null());
    let content = &resp["result"]["content"];
    assert_eq!(content[0]["type"], "text");
    assert!(resp["result"]["isError"].as_bool().unwrap_or(false));
}

#[test]
fn mcp_unknown_method() {
    let resp = send_jsonrpc("bogus/method", None);

    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);
}

#[test]
fn mcp_missing_ftl_source() {
    let resp = send_jsonrpc("tools/call", Some(serde_json::json!({
        "name": "flux_check",
        "arguments": {}
    })));

    assert!(resp["error"].is_null());
    assert!(resp["result"]["isError"].as_bool().unwrap_or(false));
}

#[test]
fn mcp_multiple_requests() {
    let req1 = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
    let req2 = serde_json::json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"});
    let input = format!("{}\n{}\n", req1, req2);

    let (stdout, _) = send_request(&input);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(lines.len(), 2, "should get 2 responses for 2 requests");

    let resp1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    let resp2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
    assert_eq!(resp1["id"], 1);
    assert_eq!(resp2["id"], 2);
}

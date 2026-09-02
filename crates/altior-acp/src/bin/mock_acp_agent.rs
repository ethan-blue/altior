//! Deterministic in-repo fixture child process for ACP runtime testing (P1.2 / P1.4).
//!
//! Speaks ACP v1 JSON-RPC over stdin/stdout without timers, sleeps, or network.
//! Controlled by the `ALTIOR_ACP_MOCK_SCENARIO` environment variable, `--scenario` arg,
//! or binary executable filename matching.

use std::io::{BufRead, BufReader, Write as _};

const SECRET_CANARY: &str = "SK_FIXTURE_TOP_SECRET_CANARY_VALUE_999";

fn main() {
    let scenario = std::env::var("ALTIOR_ACP_MOCK_SCENARIO")
        .or_else(|_| {
            let mut args = std::env::args().skip(1);
            while let Some(arg) = args.next() {
                if arg == "--scenario"
                    && let Some(val) = args.next()
                {
                    return Ok(val);
                }
            }
            Err(std::env::VarError::NotPresent)
        })
        .or_else(|_| {
            if let Ok(exe_path) = std::env::current_exe() {
                let file_name = exe_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                if file_name.contains("agent_a")
                    || file_name.contains("agent-a")
                    || file_name.contains("agent_full")
                {
                    return Ok("agent_a_full".to_owned());
                }
                if file_name.contains("agent_b")
                    || file_name.contains("agent-b")
                    || file_name.contains("agent_minimal")
                {
                    return Ok("agent_b_minimal".to_owned());
                }
                if file_name.contains("permission") {
                    return Ok("permission_flow".to_owned());
                }
                if file_name.contains("cancel") {
                    return Ok("cancel_flow".to_owned());
                }
                if file_name.contains("crash") || file_name.contains("unexpected_exit") {
                    return Ok("unexpected_exit".to_owned());
                }
                if file_name.contains("malformed") {
                    return Ok("malformed_frame".to_owned());
                }
                if file_name.contains("secret") {
                    return Ok("secret_check".to_owned());
                }
            }
            Err(std::env::VarError::NotPresent)
        })
        .unwrap_or_else(|_| "prompt_streaming".to_owned());

    run_scenario(&scenario);
}

fn extract_prompt_text(val: &serde_json::Value) -> String {
    let mut out = String::new();
    if let Some(blocks) = val
        .get("params")
        .and_then(|p| p.get("prompt"))
        .and_then(|pr| pr.as_array())
    {
        for block in blocks {
            if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                out.push_str(text);
            }
        }
    }
    out
}

#[allow(clippy::too_many_lines)]
fn run_agent_a_full<R: BufRead, W: std::io::Write, E: std::io::Write>(
    reader: &mut R,
    stdout: &mut W,
    stderr: &mut E,
) {
    if std::env::var("ALTIOR_REQUIRE_SECRET").as_deref() == Ok("1")
        && std::env::var("ALTIOR_TEST_SECRET").as_deref() != Ok(SECRET_CANARY)
    {
        let _ = stderr.write_all(b"secret missing or mismatched\n");
        let _ = stderr.flush();
        std::process::exit(1);
    }

    let mut active_session_id = "mock-session-a".to_owned();

    // 1. initialize
    let Some(line) = read_line_opt(reader) else {
        return;
    };
    let id = extract_id(&line);
    writeln_flush(
        stdout,
        &format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true,"steer":false}}}}}}"#
        ),
    );

    // Multi-request loop
    while let Some(line) = read_line_opt(reader) {
        let val: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => return,
        };

        let req_id = extract_id(&line);
        let method = val.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "session/new" => {
                active_session_id = String::from("mock-session-a");
                writeln_flush(
                    stdout,
                    &format!(
                        r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"sessionId":"{active_session_id}"}}}}"#
                    ),
                );
            }
            "session/load" => {
                if let Some(sess_id) = val
                    .get("params")
                    .and_then(|p| p.get("sessionId"))
                    .and_then(|s| s.as_str())
                {
                    active_session_id = String::from(sess_id);
                }
                writeln_flush(
                    stdout,
                    &format!(
                        r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"sessionId":"{active_session_id}"}}}}"#
                    ),
                );
            }
            "session/prompt" => {
                let prompt_text = extract_prompt_text(&val);

                if prompt_text.contains("[TRIGGER_SECRET_CHECK]")
                    && std::env::var("ALTIOR_REQUIRE_SECRET").as_deref() == Ok("1")
                    && std::env::var("ALTIOR_TEST_SECRET").as_deref() != Ok(SECRET_CANARY)
                {
                    let _ = stderr.write_all(b"secret missing or mismatched\n");
                    let _ = stderr.flush();
                    std::process::exit(1);
                }

                if prompt_text.contains("[TRIGGER_PERMISSION]")
                    || prompt_text.to_lowercase().contains("permission")
                {
                    // Send permission request to client
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":77,"method":"session/request_permission","params":{{"sessionId":"{active_session_id}","toolCall":{{"toolCallId":"tc-perm-1","status":"pending"}},"options":[{{"optionId":"allow","name":"Allow"}}]}}}}"#
                        ),
                    );

                    // Read client's answer to permission request
                    let Some(_answer) = read_line_opt(reader) else {
                        return;
                    };

                    // Emit delta
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{active_session_id}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Permission granted. Done."}}}}}}}}"#
                        ),
                    );

                    // Finish prompt turn
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"stopReason":"end_turn"}}}}"#
                        ),
                    );
                } else {
                    // Normal prompt streaming
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{active_session_id}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Agent A response: {prompt_text}"}}}}}}}}"#
                        ),
                    );
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"stopReason":"end_turn"}}}}"#
                        ),
                    );
                }
            }
            // `session/cancel` (one-way notification) and unknown methods are ignored.
            _ => {}
        }
    }
}

fn run_agent_b_minimal<R: BufRead, W: std::io::Write>(reader: &mut R, stdout: &mut W) {
    let mut active_session_id = "mock-session-b".to_owned();

    // 1. initialize: loadSession = false
    let Some(line) = read_line_opt(reader) else {
        return;
    };
    let id = extract_id(&line);
    writeln_flush(
        stdout,
        &format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":false,"steer":false}}}}}}"#
        ),
    );

    // Multi-request loop
    while let Some(line) = read_line_opt(reader) {
        let val: serde_json::Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(_) => return,
        };

        let req_id = extract_id(&line);
        let method = val.get("method").and_then(|m| m.as_str()).unwrap_or("");

        match method {
            "session/new" => {
                active_session_id = String::from("mock-session-b");
                writeln_flush(
                    stdout,
                    &format!(
                        r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"sessionId":"{active_session_id}"}}}}"#
                    ),
                );
            }
            "session/load" => {
                // session/load not supported for agent B
                writeln_flush(
                    stdout,
                    &format!(
                        r#"{{"jsonrpc":"2.0","id":{req_id},"error":{{"code":-32601,"message":"session/load not supported"}}}}"#
                    ),
                );
            }
            "session/prompt" => {
                let prompt_text = extract_prompt_text(&val);

                if prompt_text.contains("[TRIGGER_CRASH]")
                    || prompt_text.to_lowercase().contains("crash")
                {
                    // Emit first delta then exit(42)
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{active_session_id}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"About to crash..."}}}}}}}}"#
                        ),
                    );
                    std::process::exit(42);
                } else if prompt_text.contains("[TRIGGER_CANCEL]")
                    || prompt_text.to_lowercase().contains("cancel")
                {
                    // Emit first delta then wait for cancel notification without sleep
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{active_session_id}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Working on cancelable task..."}}}}}}}}"#
                        ),
                    );

                    // Wait for cancel notification (session/cancel)
                    loop {
                        let Some(cancel_line) = read_line_opt(reader) else {
                            return;
                        };
                        if let Ok(c_val) = serde_json::from_str::<serde_json::Value>(&cancel_line)
                            && let Some(c_method) = c_val.get("method").and_then(|m| m.as_str())
                            && (c_method == "session/cancel" || cancel_line.contains("cancel"))
                        {
                            break;
                        }
                    }

                    // Respond with cancelled
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"stopReason":"cancelled"}}}}"#
                        ),
                    );
                } else {
                    // Normal prompt streaming
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"{active_session_id}","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"Agent B response: {prompt_text}"}}}}}}}}"#
                        ),
                    );
                    writeln_flush(
                        stdout,
                        &format!(
                            r#"{{"jsonrpc":"2.0","id":{req_id},"result":{{"stopReason":"end_turn"}}}}"#
                        ),
                    );
                }
            }
            // `session/cancel` (one-way notification) and unknown methods are ignored.
            _ => {}
        }
    }
}

fn run_prompt_streaming_protocol<R: BufRead, W: std::io::Write>(reader: &mut R, stdout: &mut W) {
    // 1. initialize
    let line = read_line(reader);
    let id = extract_id(&line);
    writeln_flush(
        stdout,
        &format!(
            r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true,"steer":false}}}}}}"#
        ),
    );

    // 2. session/new
    let line = read_line(reader);
    let id = extract_id(&line);
    writeln_flush(
        stdout,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"mock-session-1"}}}}"#),
    );

    // 3. session/prompt
    let line = read_line(reader);
    let id = extract_id(&line);
    writeln_flush(
        stdout,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Hello"}}}}"#,
    );
    writeln_flush(
        stdout,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":" "}}}}"#,
    );
    writeln_flush(
        stdout,
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"World!"}}}}"#,
    );
    writeln_flush(
        stdout,
        &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"stopReason":"end_turn"}}}}"#),
    );
}

#[allow(clippy::too_many_lines)]
fn run_scenario(scenario: &str) {
    let stdin = std::io::stdin();
    let mut reader = BufReader::new(stdin.lock());
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();

    match scenario {
        "agent_a_full" => {
            run_agent_a_full(&mut reader, &mut stdout, &mut stderr);
        }

        "agent_b_minimal" => {
            run_agent_b_minimal(&mut reader, &mut stdout);
        }

        "prompt_streaming" => {
            run_prompt_streaming_protocol(&mut reader, &mut stdout);
        }

        "secret_check" => {
            let secret = std::env::var("ALTIOR_TEST_SECRET");
            match secret {
                Ok(val) if val == SECRET_CANARY => {
                    run_prompt_streaming_protocol(&mut reader, &mut stdout);
                }
                _ => {
                    let _ = stderr.write_all(b"secret missing or mismatched\n");
                    let _ = stderr.flush();
                    std::process::exit(1);
                }
            }
        }

        "permission_flow" => {
            // initialize
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true}}}}}}"#
                ),
            );

            // session/new
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"mock-session-1"}}}}"#
                ),
            );

            // session/prompt
            let line = read_line(&mut reader);
            let prompt_id = extract_id(&line);

            // Send permission request to client
            writeln_flush(
                &mut stdout,
                r#"{"jsonrpc":"2.0","id":77,"method":"session/request_permission","params":{"sessionId":"mock-session-1","toolCall":{"toolCallId":"tc-perm-1","status":"pending"},"options":[{"optionId":"allow","name":"Allow"}]}}"#,
            );

            // Read client's answer to permission request
            let _answer = read_line(&mut reader);

            // Finish prompt turn
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{prompt_id},"result":{{"stopReason":"end_turn"}}}}"#
                ),
            );
        }

        "cancel_flow" => {
            // initialize
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true}}}}}}"#
                ),
            );

            // session/new
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"mock-session-1"}}}}"#
                ),
            );

            // session/prompt
            let line = read_line(&mut reader);
            let prompt_id = extract_id(&line);

            // Emit a chunk so consumer receives an event and can trigger cancellation
            writeln_flush(
                &mut stdout,
                r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-session-1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Working..."}}}}"#,
            );

            // Read cancel notification
            let _cancel_notification = read_line(&mut reader);

            // Finish prompt with cancelled stop reason
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{prompt_id},"result":{{"stopReason":"cancelled"}}}}"#
                ),
            );
        }

        "malformed_frame" => {
            let _line = read_line(&mut reader);
            writeln_flush(&mut stdout, r#"{"jsonrpc":"2.0", incomplete json"#);
        }

        "oversized_line" => {
            let _line = read_line(&mut reader);
            let huge = "x".repeat(1024 * 1024 + 128);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":1,"result":{{"pad":"{huge}"}}}}"#),
            );
        }

        "unexpected_exit" => {
            // initialize
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1}}}}"#),
            );

            // session/new
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"mock-s1"}}}}"#),
            );

            // On prompt, crash with exit code 42
            let _line = read_line(&mut reader);
            std::process::exit(42);
        }

        "stderr_capture_capped" => {
            // Write 200 KiB of stderr logs
            let chunk = "E".repeat(1000) + "\n";
            for _ in 0..200 {
                let _ = stderr.write_all(chunk.as_bytes());
            }
            let _ = stderr.flush();

            // initialize
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true}}}}}}"#
                ),
            );

            // session/new
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"mock-s1"}}}}"#),
            );

            // session/prompt
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"stopReason":"end_turn"}}}}"#),
            );
        }

        "graceful_close" => {
            // initialize
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1}}}}"#),
            );

            // Wait for stdin EOF
            let mut buf = String::new();
            while let Ok(n) = reader.read_line(&mut buf) {
                if n == 0 {
                    break;
                }
                buf.clear();
            }
            std::process::exit(0);
        }

        "unknown_notification_preservation" => {
            // initialize
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(
                    r#"{{"jsonrpc":"2.0","id":{id},"result":{{"protocolVersion":1,"agentCapabilities":{{"loadSession":true}}}}}}"#
                ),
            );

            // session/new
            let line = read_line(&mut reader);
            let id = extract_id(&line);
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"sessionId":"mock-s1"}}}}"#),
            );

            // session/prompt
            let line = read_line(&mut reader);
            let id = extract_id(&line);

            // Unknown top-level notification
            writeln_flush(
                &mut stdout,
                r#"{"jsonrpc":"2.0","method":"agent/custom_telemetry","params":{"battery":98,"gpu_temp":45}}"#,
            );

            // Unknown session update kind
            writeln_flush(
                &mut stdout,
                r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"mock-s1","update":{"sessionUpdate":"neural_memory_checkpoint","layer":12}}}"#,
            );

            // Turn completed
            writeln_flush(
                &mut stdout,
                &format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{{"stopReason":"end_turn"}}}}"#),
            );
        }

        other => {
            eprintln!("unknown scenario: {other}");
            std::process::exit(1);
        }
    }
}

fn read_line_opt<R: BufRead>(reader: &mut R) -> Option<String> {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(n) if n > 0 => Some(line),
        Ok(_) | Err(_) => None,
    }
}

fn read_line<R: BufRead>(reader: &mut R) -> String {
    match read_line_opt(reader) {
        Some(line) => line,
        None => std::process::exit(0),
    }
}

fn extract_id(line: &str) -> String {
    if let Ok(val) = serde_json::from_str::<serde_json::Value>(line)
        && let Some(id) = val.get("id")
    {
        return id.to_string();
    }
    "1".to_owned()
}

fn writeln_flush<W: std::io::Write>(writer: &mut W, line: &str) {
    if writer.write_all(line.as_bytes()).is_err() {
        std::process::exit(0);
    }
    if writer.write_all(b"\n").is_err() {
        std::process::exit(0);
    }
    if writer.flush().is_err() {
        std::process::exit(0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_agent_a_full_scenario_session_and_permission() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"mock-session-a","prompt":[{"type":"text","text":"[TRIGGER_PERMISSION] run tool"}]}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":77,"result":{"optionId":"allow"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"mock-session-a","prompt":[{"type":"text","text":"normal follow up"}]}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":5,"method":"session/load","params":{"sessionId":"resumed-sess-1","cwd":"/tmp"}}"#,
            "\n"
        );

        let mut reader = BufReader::new(Cursor::new(input));
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        run_agent_a_full(&mut reader, &mut stdout, &mut stderr);

        let output_str = String::from_utf8(stdout).unwrap();
        assert!(output_str.contains(r#""loadSession":true"#));
        assert!(output_str.contains(r#""sessionId":"mock-session-a""#));
        assert!(output_str.contains(r#""method":"session/request_permission""#));
        assert!(output_str.contains(r#""Permission granted. Done.""#));
        assert!(output_str.contains(r#""Agent A response: normal follow up""#));
        assert!(output_str.contains(r#""sessionId":"resumed-sess-1""#));
    }

    #[test]
    fn test_agent_b_minimal_scenario_cancel_and_prompt() {
        let input = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":1}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"session/new","params":{"cwd":"/tmp"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":3,"method":"session/prompt","params":{"sessionId":"mock-session-b","prompt":[{"type":"text","text":"[TRIGGER_CANCEL] long task"}]}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"session/cancel","params":{"sessionId":"mock-session-b"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":4,"method":"session/prompt","params":{"sessionId":"mock-session-b","prompt":[{"type":"text","text":"normal task"}]}}"#,
            "\n"
        );

        let mut reader = BufReader::new(Cursor::new(input));
        let mut stdout = Vec::new();

        run_agent_b_minimal(&mut reader, &mut stdout);

        let output_str = String::from_utf8(stdout).unwrap();
        assert!(output_str.contains(r#""loadSession":false"#));
        assert!(output_str.contains(r#""sessionId":"mock-session-b""#));
        assert!(output_str.contains(r#""Working on cancelable task...""#));
        assert!(output_str.contains(r#""stopReason":"cancelled""#));
        assert!(output_str.contains(r#""Agent B response: normal task""#));
    }
}

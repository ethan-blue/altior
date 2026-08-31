//! Deterministic in-repo fixture child process for ACP runtime testing (P1.2).
//!
//! Speaks ACP v1 JSON-RPC over stdin/stdout without timers, sleeps, or network.
//! Controlled by the `ALTIOR_ACP_MOCK_SCENARIO` environment variable or `--scenario` arg.

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

fn read_line<R: BufRead>(reader: &mut R) -> String {
    let mut line = String::new();
    match reader.read_line(&mut line) {
        Ok(0) => {
            // Parent closed stdin (EOF) or disconnected. Exit immediately.
            std::process::exit(0);
        }
        Ok(_) => line,
        Err(_) => {
            // Read error (e.g. broken pipe). Exit immediately.
            std::process::exit(0);
        }
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

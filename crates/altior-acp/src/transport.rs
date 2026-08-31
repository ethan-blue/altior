//! Subprocess transport port and `AcpChild` management (ADR 0007).
//!
//! Spawns ACP agent child processes using strict `program` + `args` arrays
//! (no shell concatenation), handles newline-delimited JSON-RPC over stdin/stdout,
//! captures bounded stderr diagnostics (up to [`MAX_STDERR_CAPTURE_BYTES`]),
//! and supports graceful close and termination.
//!
//! Conforms to repository dependency boundaries: standard library `std::process`
//! is used without external runtime dependencies.

use std::fmt;
use std::io::{BufReader, Read, Write as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::config::{MAX_STDERR_CAPTURE_BYTES, ResolvedLaunchConfig};
use crate::error::AcpError;
use crate::wire::{MAX_LINE_BYTES, encode_line};

/// Trait defining the process transport port for an ACP agent subprocess.
pub trait ProcessTransport: Send {
    /// Writes a single JSON-RPC line (with newline delimiter) to stdin.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::LineTooLarge`] if the line exceeds [`MAX_LINE_BYTES`],
    /// or [`AcpError::IoError`] / [`AcpError::ProcessExited`] on pipe failures.
    fn write_line(&mut self, line: &str) -> Result<(), AcpError>;

    /// Reads the next newline-delimited line from stdout.
    ///
    /// Returns `Ok(Some(line))` on success, or `Ok(None)` on EOF.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::LineTooLarge`] if the line exceeds [`MAX_LINE_BYTES`],
    /// [`AcpError::LineNotUtf8`] on invalid UTF-8, or [`AcpError::IoError`].
    fn read_line(&mut self) -> Result<Option<String>, AcpError>;

    /// Returns a snapshot of the captured stderr output (bounded).
    fn captured_stderr(&self) -> String;

    /// Checks if the subprocess has exited without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::IoError`] if status check fails.
    fn try_wait(&mut self) -> Result<Option<ExitStatus>, AcpError>;

    /// Forcefully terminates the subprocess.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::IoError`] if termination fails.
    fn terminate(&mut self) -> Result<(), AcpError>;

    /// Gracefully closes stdin and waits for child termination.
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::IoError`] on failure.
    fn close(&mut self) -> Result<Option<ExitStatus>, AcpError>;

    /// The process identifier (PID).
    fn pid(&self) -> u32;
}

/// A running ACP agent child process.
pub struct AcpChild {
    child: Child,
    stdin: Option<std::process::ChildStdin>,
    stdout_reader: Option<BufReader<std::process::ChildStdout>>,
    stderr_buffer: Arc<Mutex<Vec<u8>>>,
    stderr_thread: Option<JoinHandle<()>>,
    pid: u32,
}

impl AcpChild {
    /// Spawns a new ACP agent subprocess from a resolved launch configuration.
    ///
    /// Shell concatenation is prohibited: `program` and `args` are invoked
    /// directly via [`std::process::Command`].
    ///
    /// # Errors
    ///
    /// Returns [`AcpError::ProcessSpawnFailed`] if the child cannot be started.
    pub fn spawn(config: &ResolvedLaunchConfig) -> Result<Self, AcpError> {
        let mut command = Command::new(&config.program);
        command.args(&config.args);
        if let Some(cwd) = &config.working_dir {
            command.current_dir(cwd);
        }
        command.envs(&config.env);
        command.stdin(Stdio::piped());
        command.stdout(Stdio::piped());
        command.stderr(Stdio::piped());

        let mut child = command
            .spawn()
            .map_err(|err| AcpError::ProcessSpawnFailed {
                program: config.program.clone(),
                diagnostic: err.to_string(),
            })?;

        let pid = child.id();
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();

        let stdout_reader = stdout.map(BufReader::new);

        let stderr_buffer = Arc::new(Mutex::new(Vec::new()));
        let stderr_thread = stderr.map(|mut stderr_stream| {
            let buffer_ref = Arc::clone(&stderr_buffer);
            std::thread::spawn(move || {
                let mut chunk = [0u8; 4096];
                let mut total_read = 0;
                while let Ok(n) = stderr_stream.read(&mut chunk) {
                    if n == 0 {
                        break;
                    }
                    let mut buf = match buffer_ref.lock() {
                        Ok(b) => b,
                        Err(poisoned) => poisoned.into_inner(),
                    };
                    let available = MAX_STDERR_CAPTURE_BYTES.saturating_sub(total_read);
                    if available > 0 {
                        let to_append = n.min(available);
                        buf.extend_from_slice(&chunk[..to_append]);
                        total_read += to_append;
                        if total_read >= MAX_STDERR_CAPTURE_BYTES {
                            buf.extend_from_slice(b"\n...[stderr truncated]");
                        }
                    }
                }
            })
        });

        Ok(Self {
            child,
            stdin,
            stdout_reader,
            stderr_buffer,
            stderr_thread,
            pid,
        })
    }
}

impl ProcessTransport for AcpChild {
    fn write_line(&mut self, line: &str) -> Result<(), AcpError> {
        let bytes = encode_line(line)?;
        let stdin = self.stdin.as_mut().ok_or_else(|| AcpError::ProcessExited {
            status: "stdin pipe already closed".to_owned(),
        })?;

        stdin.write_all(&bytes).map_err(|err| {
            if let Ok(Some(status)) = self.child.try_wait() {
                AcpError::ProcessExited {
                    status: format!("child exited with status {status}"),
                }
            } else {
                AcpError::IoError {
                    diagnostic: format!("failed to write to child stdin: {err}"),
                }
            }
        })?;

        stdin.flush().map_err(|err| AcpError::IoError {
            diagnostic: format!("failed to flush child stdin: {err}"),
        })
    }

    fn read_line(&mut self) -> Result<Option<String>, AcpError> {
        let reader = self
            .stdout_reader
            .as_mut()
            .ok_or_else(|| AcpError::ProcessExited {
                status: "stdout pipe already closed".to_owned(),
            })?;

        let mut line_bytes = Vec::new();
        let mut total_read = 0;

        loop {
            let mut byte_buf = [0u8; 1];
            match reader.read(&mut byte_buf) {
                Ok(0) => {
                    if line_bytes.is_empty() {
                        return Ok(None);
                    }
                    break;
                }
                Ok(1) => {
                    let b = byte_buf[0];
                    total_read += 1;
                    if total_read > MAX_LINE_BYTES {
                        return Err(AcpError::LineTooLarge {
                            size_bytes: total_read,
                            limit_bytes: MAX_LINE_BYTES,
                        });
                    }
                    if b == b'\n' {
                        break;
                    }
                    line_bytes.push(b);
                }
                Ok(_) => unreachable!(),
                Err(err) if err.kind() == std::io::ErrorKind::Interrupted => {}
                Err(err) => {
                    return Err(AcpError::IoError {
                        diagnostic: format!("failed to read line from child stdout: {err}"),
                    });
                }
            }
        }

        // Strip trailing \r if on Windows CRLF stream
        if line_bytes.last() == Some(&b'\r') {
            line_bytes.pop();
        }

        let line = String::from_utf8(line_bytes).map_err(|_| AcpError::LineNotUtf8)?;
        Ok(Some(line))
    }

    fn captured_stderr(&self) -> String {
        let buf = match self.stderr_buffer.lock() {
            Ok(b) => b,
            Err(poisoned) => poisoned.into_inner(),
        };
        String::from_utf8_lossy(&buf).into_owned()
    }

    fn try_wait(&mut self) -> Result<Option<ExitStatus>, AcpError> {
        self.child.try_wait().map_err(|err| AcpError::IoError {
            diagnostic: format!("failed to query child status: {err}"),
        })
    }

    fn terminate(&mut self) -> Result<(), AcpError> {
        let _ = self.stdin.take();
        let _ = self.stdout_reader.take();
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
        Ok(())
    }

    fn close(&mut self) -> Result<Option<ExitStatus>, AcpError> {
        let _ = self.stdin.take(); // Drops stdin handle, closing the pipe.
        let status_res = self.child.wait();
        let _ = self.stdout_reader.take();
        if let Some(thread) = self.stderr_thread.take() {
            let _ = thread.join();
        }
        let status = status_res.map_err(|err| AcpError::IoError {
            diagnostic: format!("failed waiting for child exit on close: {err}"),
        })?;
        Ok(Some(status))
    }

    fn pid(&self) -> u32 {
        self.pid
    }
}

impl fmt::Debug for AcpChild {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AcpChild")
            .field("pid", &self.pid)
            .field("stdin_open", &self.stdin.is_some())
            .finish_non_exhaustive()
    }
}

impl Drop for AcpChild {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

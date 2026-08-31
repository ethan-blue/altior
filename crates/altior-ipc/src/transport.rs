//! Real local OS transport for IPC (ADR 0006).
//!
//! Provides [`LocalEndpoint`], [`LocalListener`], and [`LocalStream`] abstractions.
//! Uses Windows Named Pipes on Windows and Unix Domain Sockets on Unix.
//! Framing enforces the 256 KiB [`MAX_FRAME_BYTES`] cap with zero unbounded reads.

use std::io::{Read, Write};
use std::time::Duration;

use crate::endpoint::LocalEndpoint;
use crate::error::IpcError;
use crate::frame::{FrameDecoder, encode_frame};

/// Listener for incoming local IPC client connections.
///
/// Supports multiple sequential or concurrent clients. When a client disconnects,
/// the listener remains open and continues accepting new connections.
#[derive(Debug)]
pub struct LocalListener {
    endpoint: LocalEndpoint,
    inner: PlatformListener,
}

impl LocalListener {
    /// Binds a listener on the specified local endpoint.
    ///
    /// # Errors
    /// Returns [`IpcError::InvalidEndpoint`] on malformed address,
    /// [`IpcError::AccessDenied`] if the pipe already exists or permission denied,
    /// or [`IpcError::Io`] on platform binding failure.
    pub fn bind(endpoint: &LocalEndpoint) -> Result<Self, IpcError> {
        match endpoint {
            LocalEndpoint::WindowsPipe(name) => {
                #[cfg(windows)]
                {
                    if !name.starts_with(r"\\.\pipe\") {
                        return Err(IpcError::InvalidEndpoint {
                            reason: format!(
                                "Windows pipe name must start with \\\\.\\pipe\\: {name}"
                            ),
                        });
                    }
                    if name.len() > 256 {
                        return Err(IpcError::InvalidEndpoint {
                            reason: format!("Windows pipe name exceeds 256 bytes: {name}"),
                        });
                    }
                    let wide_name = encode_wide(name);
                    let initial_pipe = create_pipe_instance(&wide_name, true)?;
                    let listener = WindowsNamedPipeListener::new(wide_name, initial_pipe)?;
                    Ok(Self {
                        endpoint: endpoint.clone(),
                        inner: PlatformListener::Windows(listener),
                    })
                }
                #[cfg(not(windows))]
                {
                    let _ = name;
                    Err(IpcError::InvalidEndpoint {
                        reason: "Windows named pipes are not supported on non-Windows platforms"
                            .to_owned(),
                    })
                }
            }
            LocalEndpoint::UnixSocket(path) => {
                #[cfg(unix)]
                {
                    if path.is_empty() {
                        return Err(IpcError::InvalidEndpoint {
                            reason: "Unix socket path cannot be empty".to_owned(),
                        });
                    }
                    if path.len() > 104 {
                        return Err(IpcError::InvalidEndpoint {
                            reason: format!("Unix socket path exceeds 104 bytes: {path}"),
                        });
                    }
                    let listener = bind_unix_socket(path)?;
                    Ok(Self {
                        endpoint: endpoint.clone(),
                        inner: PlatformListener::Unix(listener),
                    })
                }
                #[cfg(not(unix))]
                {
                    let _ = path;
                    Err(IpcError::InvalidEndpoint {
                        reason: "Unix domain sockets are not supported on Windows".to_owned(),
                    })
                }
            }
        }
    }

    /// Accepts an incoming client connection, blocking until one is available.
    ///
    /// # Errors
    /// Returns [`IpcError`] if accepting fails.
    pub fn accept(&self) -> Result<LocalStream, IpcError> {
        match &self.inner {
            #[cfg(windows)]
            PlatformListener::Windows(listener) => listener.accept(),
            #[cfg(unix)]
            PlatformListener::Unix(listener) => {
                let (stream, _) = listener.listener.accept()?;
                Ok(LocalStream::new_unix(stream))
            }
        }
    }

    /// Tries to accept an incoming client connection without blocking.
    ///
    /// # Errors
    /// Returns [`IpcError`] on accept error.
    pub fn try_accept(&self) -> Result<Option<LocalStream>, IpcError> {
        match &self.inner {
            #[cfg(windows)]
            PlatformListener::Windows(listener) => listener.try_accept(),
            #[cfg(unix)]
            PlatformListener::Unix(listener) => match listener.listener.accept() {
                Ok((stream, _)) => Ok(Some(LocalStream::new_unix(stream))),
                Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                Err(err) => Err(IpcError::from(err)),
            },
        }
    }

    /// Polls for an incoming client connection with a timeout.
    ///
    /// # Errors
    /// Returns [`IpcError`] on accept error.
    pub fn poll_accept(&self, timeout: Duration) -> Result<Option<LocalStream>, IpcError> {
        match &self.inner {
            #[cfg(windows)]
            PlatformListener::Windows(listener) => listener.poll_accept(timeout),
            #[cfg(unix)]
            PlatformListener::Unix(_) => {
                let start = std::time::Instant::now();
                loop {
                    if let Some(stream) = self.try_accept()? {
                        return Ok(Some(stream));
                    }
                    if start.elapsed() >= timeout {
                        return Ok(None);
                    }
                    std::thread::yield_now();
                }
            }
        }
    }

    /// The local endpoint this listener is listening on.
    #[must_use]
    pub fn endpoint(&self) -> &LocalEndpoint {
        &self.endpoint
    }
}

/// A connected bidirectional local stream.
///
/// Implements [`Read`] and [`Write`], with framed helpers enforcing
/// the length-prefixed protocol contract without unbounded reads.
#[derive(Debug)]
pub struct LocalStream {
    inner: PlatformStream,
    decoder: FrameDecoder,
    closed: bool,
}

impl Read for LocalStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.inner.read(buf)
    }
}

impl Write for LocalStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.inner.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

impl LocalStream {
    #[cfg(windows)]
    pub(crate) fn new_windows(file: std::fs::File) -> Self {
        Self {
            inner: PlatformStream::Windows(file),
            decoder: FrameDecoder::new(),
            closed: false,
        }
    }

    #[cfg(unix)]
    pub(crate) fn new_unix(stream: std::os::unix::net::UnixStream) -> Self {
        let _ = stream.set_nonblocking(true);
        Self {
            inner: PlatformStream::Unix(stream),
            decoder: FrameDecoder::new(),
            closed: false,
        }
    }

    /// Connects to a local endpoint with an optional timeout.
    ///
    /// # Errors
    /// Returns [`IpcError::NotFound`] if the endpoint does not exist,
    /// [`IpcError::AccessDenied`] on permission failure,
    /// [`IpcError::Timeout`] if connection timed out,
    /// or [`IpcError::Io`] on platform failure.
    pub fn connect(endpoint: &LocalEndpoint, timeout: Option<Duration>) -> Result<Self, IpcError> {
        match endpoint {
            LocalEndpoint::WindowsPipe(name) => {
                #[cfg(windows)]
                {
                    connect_windows_named_pipe(name, timeout)
                }
                #[cfg(not(windows))]
                {
                    let _ = (name, timeout);
                    Err(IpcError::InvalidEndpoint {
                        reason: "Windows named pipes are not supported on non-Windows platforms"
                            .to_owned(),
                    })
                }
            }
            LocalEndpoint::UnixSocket(path) => {
                #[cfg(unix)]
                {
                    connect_unix_socket(path, timeout)
                }
                #[cfg(not(unix))]
                {
                    let _ = (path, timeout);
                    Err(IpcError::InvalidEndpoint {
                        reason: "Unix domain sockets are not supported on Windows".to_owned(),
                    })
                }
            }
        }
    }

    /// Clones this stream handle.
    ///
    /// # Errors
    /// Returns [`IpcError::Io`] if duplicating the underlying handle fails.
    pub fn try_clone(&self) -> Result<Self, IpcError> {
        match &self.inner {
            #[cfg(windows)]
            PlatformStream::Windows(file) => {
                let cloned = file.try_clone()?;
                Ok(Self {
                    inner: PlatformStream::Windows(cloned),
                    decoder: FrameDecoder::new(),
                    closed: self.closed,
                })
            }
            #[cfg(unix)]
            PlatformStream::Unix(stream) => {
                let cloned = stream.try_clone()?;
                Ok(Self {
                    inner: PlatformStream::Unix(cloned),
                    decoder: FrameDecoder::new(),
                    closed: self.closed,
                })
            }
        }
    }

    /// Sends a length-prefixed frame containing the given UTF-8 payload.
    ///
    /// # Errors
    /// Returns [`IpcError::FrameTooLarge`] if `payload` exceeds [`MAX_FRAME_BYTES`],
    /// or [`IpcError::Io`] if writing fails.
    pub fn send_frame(&mut self, payload: &str) -> Result<(), IpcError> {
        let frame = encode_frame(payload)?;
        self.send_raw_frame(&frame)
    }

    /// Sends raw pre-encoded frame bytes.
    ///
    /// # Errors
    /// Returns [`IpcError::Io`] if writing fails.
    pub fn send_raw_frame(&mut self, frame: &[u8]) -> Result<(), IpcError> {
        if self.closed {
            return Err(IpcError::SessionOrder {
                attempted: "send raw frame over closed stream",
            });
        }
        self.write_all(frame)?;
        self.flush()?;
        Ok(())
    }

    /// Receives one length-prefixed frame payload without unbounded reading, blocking until available.
    ///
    /// Reads length prefix first. If the declared length exceeds
    /// [`MAX_FRAME_BYTES`], returns [`IpcError::FrameTooLarge`] immediately without
    /// reading the payload.
    ///
    /// # Errors
    /// Returns [`IpcError::FrameTooLarge`] if declared frame length exceeds 256 KiB,
    /// [`IpcError::EndpointUnavailable`] on clean EOF, or [`IpcError::Io`].
    pub fn recv_frame(&mut self) -> Result<Vec<u8>, IpcError> {
        loop {
            if let Some(frame) = self.try_recv_frame()? {
                return Ok(frame);
            }
            std::thread::yield_now();
            std::thread::sleep(Duration::from_millis(1));
        }
    }

    /// Tries to receive one length-prefixed frame payload non-blockingly.
    ///
    /// Returns `Ok(Some(frame))` if a complete frame is decoded,
    /// `Ok(None)` if no complete frame is available right now,
    /// or `Err(IpcError)` on connection error / EOF.
    ///
    /// # Errors
    /// Returns [`IpcError::FrameTooLarge`] if declared frame length exceeds 256 KiB,
    /// [`IpcError::EndpointUnavailable`] on connection close, or [`IpcError::Io`].
    // The small overage keeps the Windows and Unix non-blocking read branches
    // adjacent so their EOF/error semantics remain directly comparable.
    #[allow(clippy::too_many_lines)]
    pub fn try_recv_frame(&mut self) -> Result<Option<Vec<u8>>, IpcError> {
        if self.closed {
            return Err(IpcError::EndpointUnavailable {
                endpoint: "stream closed".to_owned(),
            });
        }

        // Check if decoder already has complete frames ready
        if let Some(frame) = self.decoder.decode_next()? {
            return Ok(Some(frame));
        }

        match &mut self.inner {
            #[cfg(windows)]
            PlatformStream::Windows(file) => {
                use std::os::windows::io::AsRawHandle;
                use windows_sys::Win32::Foundation::{
                    ERROR_BROKEN_PIPE, ERROR_HANDLE_EOF, ERROR_NO_DATA, ERROR_PIPE_NOT_CONNECTED,
                    GetLastError,
                };
                use windows_sys::Win32::System::Pipes::PeekNamedPipe;

                let handle = file.as_raw_handle();
                let mut bytes_avail: u32 = 0;
                let peek_res = unsafe {
                    PeekNamedPipe(
                        handle.cast(),
                        std::ptr::null_mut(),
                        0,
                        std::ptr::null_mut(),
                        &raw mut bytes_avail,
                        std::ptr::null_mut(),
                    )
                };

                if peek_res == 0 {
                    let err = unsafe { GetLastError() };
                    if err == ERROR_BROKEN_PIPE
                        || err == ERROR_PIPE_NOT_CONNECTED
                        || err == ERROR_HANDLE_EOF
                        || err == ERROR_NO_DATA
                    {
                        self.closed = true;
                        return Err(IpcError::EndpointUnavailable {
                            endpoint: "stream closed by peer".to_owned(),
                        });
                    }
                    return Err(IpcError::Io {
                        source: std::io::Error::from_raw_os_error(err.cast_signed()),
                    });
                }

                if bytes_avail == 0 {
                    return Ok(None);
                }

                let to_read = (bytes_avail as usize).clamp(1, 65536);
                let mut buf = vec![0u8; to_read];
                match file.read(&mut buf) {
                    Ok(0) => {
                        self.closed = true;
                        Err(IpcError::EndpointUnavailable {
                            endpoint: "stream closed by peer".to_owned(),
                        })
                    }
                    Ok(n) => {
                        self.decoder.feed_bytes(&buf[..n]);
                        self.decoder.decode_next()
                    }
                    Err(err)
                        if err.kind() == std::io::ErrorKind::UnexpectedEof
                            || err.kind() == std::io::ErrorKind::ConnectionReset
                            || err.kind() == std::io::ErrorKind::BrokenPipe =>
                    {
                        self.closed = true;
                        Err(IpcError::EndpointUnavailable {
                            endpoint: "stream closed by peer".to_owned(),
                        })
                    }
                    Err(err) => Err(IpcError::from(err)),
                }
            }
            #[cfg(unix)]
            PlatformStream::Unix(stream) => {
                let mut buf = [0u8; 8192];
                match stream.read(&mut buf) {
                    Ok(0) => {
                        self.closed = true;
                        Err(IpcError::EndpointUnavailable {
                            endpoint: "stream closed by peer".to_owned(),
                        })
                    }
                    Ok(n) => {
                        self.decoder.feed_bytes(&buf[..n]);
                        self.decoder.decode_next()
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => Ok(None),
                    Err(err)
                        if err.kind() == std::io::ErrorKind::UnexpectedEof
                            || err.kind() == std::io::ErrorKind::ConnectionReset
                            || err.kind() == std::io::ErrorKind::BrokenPipe =>
                    {
                        self.closed = true;
                        Err(IpcError::EndpointUnavailable {
                            endpoint: "stream closed by peer".to_owned(),
                        })
                    }
                    Err(err) => Err(IpcError::from(err)),
                }
            }
        }
    }

    /// Receives one frame and decodes it as a UTF-8 string.
    ///
    /// # Errors
    /// Returns [`IpcError::FrameNotUtf8`] if payload is not valid UTF-8,
    /// or other framing/I/O errors.
    pub fn recv_frame_string(&mut self) -> Result<String, IpcError> {
        let bytes = self.recv_frame()?;
        String::from_utf8(bytes).map_err(|_| IpcError::FrameNotUtf8)
    }

    /// Sends a JSON-serializable value as a frame.
    ///
    /// # Errors
    /// Returns [`IpcError::Protocol`] on serialization failure, or I/O errors.
    pub fn send_json<T: serde::Serialize>(&mut self, value: &T) -> Result<(), IpcError> {
        let json = serde_json::to_string(value).map_err(|err| IpcError::Protocol {
            source: altior_protocol::ProtocolError::MalformedEnvelope { source: err },
        })?;
        self.send_frame(&json)
    }

    /// Receives a frame and deserializes it from JSON.
    ///
    /// # Errors
    /// Returns [`IpcError::Protocol`] on deserialization failure, or I/O errors.
    pub fn recv_json<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, IpcError> {
        let text = self.recv_frame_string()?;
        serde_json::from_str(&text).map_err(|err| IpcError::Protocol {
            source: altior_protocol::ProtocolError::MalformedEnvelope { source: err },
        })
    }

    /// Explicitly closes the local stream.
    ///
    /// # Errors
    /// Returns [`IpcError`] on failure.
    pub fn close(&mut self) -> Result<(), IpcError> {
        self.closed = true;
        let _ = self.flush();
        Ok(())
    }

    /// Returns `true` if this stream has been closed.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed
    }
}

#[derive(Debug)]
enum PlatformListener {
    #[cfg(windows)]
    Windows(WindowsNamedPipeListener),
    #[cfg(unix)]
    Unix(UnixSocketListener),
}

#[derive(Debug)]
enum PlatformStream {
    #[cfg(windows)]
    Windows(std::fs::File),
    #[cfg(unix)]
    Unix(std::os::unix::net::UnixStream),
}

impl Read for PlatformStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(windows)]
            Self::Windows(file) => file.read(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.read(buf),
        }
    }
}

impl Write for PlatformStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            #[cfg(windows)]
            Self::Windows(file) => file.write(buf),
            #[cfg(unix)]
            Self::Unix(stream) => stream.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            #[cfg(windows)]
            Self::Windows(file) => file.flush(),
            #[cfg(unix)]
            Self::Unix(stream) => stream.flush(),
        }
    }
}

// ---------------------------------------------------------------------------
// Windows Named Pipe Platform Implementation
// ---------------------------------------------------------------------------

#[cfg(windows)]
#[derive(Debug)]
struct PipeHandle(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
unsafe impl Send for PipeHandle {}
#[cfg(windows)]
unsafe impl Sync for PipeHandle {}

#[cfg(windows)]
impl Drop for PipeHandle {
    fn drop(&mut self) {
        if self.0 != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE && !self.0.is_null() {
            unsafe {
                windows_sys::Win32::Foundation::CloseHandle(self.0);
            }
        }
    }
}

#[cfg(windows)]
#[derive(Debug)]
struct WindowsNamedPipeListener {
    pipe_name: Vec<u16>,
    incoming_rx: std::sync::mpsc::Receiver<Result<LocalStream, IpcError>>,
    active_handle: std::sync::Arc<std::sync::Mutex<isize>>,
}

#[cfg(windows)]
impl Drop for WindowsNamedPipeListener {
    fn drop(&mut self) {
        if let Ok(handle_val) = self.active_handle.lock() {
            let handle = (*handle_val) as windows_sys::Win32::Foundation::HANDLE;
            if handle != windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE && !handle.is_null() {
                unsafe {
                    windows_sys::Win32::System::IO::CancelIoEx(handle, std::ptr::null());
                }
            }
        }
    }
}

#[cfg(windows)]
impl WindowsNamedPipeListener {
    fn new(pipe_name: Vec<u16>, initial_pipe: PipeHandle) -> Result<Self, IpcError> {
        let initial_raw = initial_pipe.0 as isize;
        let active_handle = std::sync::Arc::new(std::sync::Mutex::new(initial_raw));
        let active_handle_clone = std::sync::Arc::clone(&active_handle);
        let (tx, rx) = std::sync::mpsc::sync_channel(32);
        let pipe_name_clone = pipe_name.clone();

        std::thread::Builder::new()
            .name("altior-pipe-accept".into())
            .spawn(move || {
                use std::os::windows::io::FromRawHandle;
                use windows_sys::Win32::Foundation::{
                    ERROR_INVALID_HANDLE, ERROR_OPERATION_ABORTED, ERROR_PIPE_CONNECTED,
                    GetLastError,
                };
                use windows_sys::Win32::System::Pipes::ConnectNamedPipe;

                let mut current_pipe = initial_pipe;
                loop {
                    let handle = current_pipe.0;
                    if let Ok(mut h) = active_handle_clone.lock() {
                        *h = handle as isize;
                    }

                    let connect_res = unsafe { ConnectNamedPipe(handle, std::ptr::null_mut()) };
                    if connect_res == 0 {
                        let err = unsafe { GetLastError() };
                        if err == ERROR_OPERATION_ABORTED || err == ERROR_INVALID_HANDLE {
                            break;
                        }
                        if err != ERROR_PIPE_CONNECTED {
                            let _ = tx.send(Err(IpcError::Io {
                                source: std::io::Error::from_raw_os_error(err.cast_signed()),
                            }));
                            break;
                        }
                    }

                    // Connection established! Create next pipe instance (first_instance = false)
                    let next_pipe = match create_pipe_instance(&pipe_name_clone, false) {
                        Ok(pipe) => pipe,
                        Err(e) => {
                            let _ = tx.send(Err(e));
                            break;
                        }
                    };

                    let connected_raw = current_pipe.0;
                    std::mem::forget(current_pipe);
                    current_pipe = next_pipe;

                    let file = unsafe { std::fs::File::from_raw_handle(connected_raw.cast()) };
                    let stream = LocalStream::new_windows(file);
                    if tx.send(Ok(stream)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| IpcError::Io { source: e })?;

        Ok(Self {
            pipe_name,
            incoming_rx: rx,
            active_handle,
        })
    }

    fn accept(&self) -> Result<LocalStream, IpcError> {
        let endpoint_str = String::from_utf16_lossy(&self.pipe_name);
        self.incoming_rx
            .recv()
            .map_err(|_| IpcError::EndpointUnavailable {
                endpoint: endpoint_str,
            })?
    }

    fn try_accept(&self) -> Result<Option<LocalStream>, IpcError> {
        let endpoint_str = String::from_utf16_lossy(&self.pipe_name);
        match self.incoming_rx.try_recv() {
            Ok(Ok(stream)) => Ok(Some(stream)),
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::TryRecvError::Empty) => Ok(None),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err(IpcError::EndpointUnavailable {
                    endpoint: endpoint_str,
                })
            }
        }
    }

    fn poll_accept(&self, timeout: Duration) -> Result<Option<LocalStream>, IpcError> {
        let endpoint_str = String::from_utf16_lossy(&self.pipe_name);
        match self.incoming_rx.recv_timeout(timeout) {
            Ok(Ok(stream)) => Ok(Some(stream)),
            Ok(Err(e)) => Err(e),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                Err(IpcError::EndpointUnavailable {
                    endpoint: endpoint_str,
                })
            }
        }
    }
}

#[cfg(windows)]
fn create_pipe_instance(wide_name: &[u16], first_instance: bool) -> Result<PipeHandle, IpcError> {
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, GetLastError, INVALID_HANDLE_VALUE, LocalFree,
    };
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
    };
    use windows_sys::Win32::Security::SECURITY_ATTRIBUTES;
    use windows_sys::Win32::System::Pipes::{
        CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_BYTE,
        PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
    };

    const PIPE_ACCESS_DUPLEX: u32 = 0x0000_0003;
    const FILE_FLAG_FIRST_PIPE_INSTANCE: u32 = 0x0008_0000;

    // SECURITY DEBT: We configure an SDDL owner/system/admin DACL and
    // PIPE_REJECT_REMOTE_CLIENTS. Full AppContainer isolation and fine-grained
    // client SID verification are deferred to P5 hardening.
    let mut sa = SECURITY_ATTRIBUTES {
        nLength: u32::try_from(std::mem::size_of::<SECURITY_ATTRIBUTES>()).unwrap_or(0),
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 0,
    };

    let sddl = encode_wide("D:(A;;GA;;;OW)(A;;GA;;;SY)(A;;GA;;;BA)");
    let mut p_sd = std::ptr::null_mut();
    let sddl_ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &raw mut p_sd,
            std::ptr::null_mut(),
        )
    };

    if sddl_ok != 0 && !p_sd.is_null() {
        sa.lpSecurityDescriptor = p_sd;
    }

    let mut open_mode = PIPE_ACCESS_DUPLEX;
    if first_instance {
        open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
    }

    let pipe_mode = PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
    let max_instances = PIPE_UNLIMITED_INSTANCES;
    let out_buffer_size = 64 * 1024;
    let in_buffer_size = 64 * 1024;
    let default_timeout = 5000;

    let handle = unsafe {
        CreateNamedPipeW(
            wide_name.as_ptr(),
            open_mode,
            pipe_mode,
            max_instances,
            out_buffer_size,
            in_buffer_size,
            default_timeout,
            if sa.lpSecurityDescriptor.is_null() {
                std::ptr::null()
            } else {
                &raw const sa
            },
        )
    };

    if !p_sd.is_null() {
        unsafe {
            LocalFree(p_sd.cast());
        }
    }

    if handle == INVALID_HANDLE_VALUE {
        let err = unsafe { GetLastError() };
        if err == ERROR_ACCESS_DENIED {
            let name_str = String::from_utf16_lossy(wide_name);
            return Err(IpcError::AccessDenied {
                endpoint: name_str.trim_matches('\0').to_owned(),
            });
        }
        return Err(IpcError::Io {
            source: std::io::Error::from_raw_os_error(err.cast_signed()),
        });
    }

    Ok(PipeHandle(handle))
}

#[cfg(windows)]
fn connect_windows_named_pipe(
    pipe_name: &str,
    timeout: Option<Duration>,
) -> Result<LocalStream, IpcError> {
    use std::os::windows::io::FromRawHandle;
    use windows_sys::Win32::Foundation::{
        ERROR_ACCESS_DENIED, ERROR_FILE_NOT_FOUND, ERROR_PATH_NOT_FOUND, ERROR_PIPE_BUSY,
        ERROR_SEM_TIMEOUT, GENERIC_READ, GENERIC_WRITE, GetLastError, INVALID_HANDLE_VALUE,
    };
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_ATTRIBUTE_NORMAL, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Pipes::WaitNamedPipeW;

    let wide_name = encode_wide(pipe_name);
    let start_time = std::time::Instant::now();
    let timeout_duration = timeout.unwrap_or(Duration::from_secs(5));

    loop {
        let handle = unsafe {
            CreateFileW(
                wide_name.as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                std::ptr::null(),
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                std::ptr::null_mut(),
            )
        };

        if handle != INVALID_HANDLE_VALUE {
            let file = unsafe { std::fs::File::from_raw_handle(handle.cast()) };
            return Ok(LocalStream::new_windows(file));
        }

        let err = unsafe { GetLastError() };
        match err {
            ERROR_FILE_NOT_FOUND | ERROR_PATH_NOT_FOUND => {
                return Err(IpcError::NotFound {
                    endpoint: pipe_name.to_owned(),
                });
            }
            ERROR_ACCESS_DENIED => {
                return Err(IpcError::AccessDenied {
                    endpoint: pipe_name.to_owned(),
                });
            }
            ERROR_PIPE_BUSY => {
                let elapsed = start_time.elapsed();
                if elapsed >= timeout_duration {
                    return Err(IpcError::Timeout {
                        endpoint: pipe_name.to_owned(),
                    });
                }
                let remaining = timeout_duration.saturating_sub(elapsed);
                let wait_ms = u32::try_from(remaining.as_millis().clamp(1, 1000)).unwrap_or(1000);
                let waited = unsafe { WaitNamedPipeW(wide_name.as_ptr(), wait_ms) };
                if waited == 0 {
                    let wait_err = unsafe { GetLastError() };
                    if wait_err == ERROR_SEM_TIMEOUT || start_time.elapsed() >= timeout_duration {
                        return Err(IpcError::Timeout {
                            endpoint: pipe_name.to_owned(),
                        });
                    }
                }
            }
            other => {
                return Err(IpcError::Io {
                    source: std::io::Error::from_raw_os_error(other.cast_signed()),
                });
            }
        }
    }
}

#[cfg(windows)]
fn encode_wide(s: &str) -> Vec<u16> {
    use std::os::windows::ffi::OsStrExt;
    std::ffi::OsStr::new(s)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

// ---------------------------------------------------------------------------
// Unix Domain Socket Platform Implementation
// ---------------------------------------------------------------------------

#[cfg(unix)]
static UMASK_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(unix)]
struct UmaskGuard {
    prev_mask: libc::mode_t,
}

#[cfg(unix)]
impl UmaskGuard {
    fn new(new_mask: libc::mode_t) -> Self {
        // SAFETY: libc::umask always succeeds and returns the previous umask.
        // It is called while holding UMASK_LOCK to prevent process-wide race conditions.
        let prev_mask = unsafe { libc::umask(new_mask) };
        Self { prev_mask }
    }
}

#[cfg(unix)]
impl Drop for UmaskGuard {
    fn drop(&mut self) {
        // SAFETY: Restoring the previous umask within the locked critical section.
        unsafe {
            libc::umask(self.prev_mask);
        }
    }
}

#[cfg(unix)]
#[derive(Debug)]
struct UnixSocketListener {
    socket_path: std::path::PathBuf,
    listener: std::os::unix::net::UnixListener,
}

#[cfg(unix)]
impl Drop for UnixSocketListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

#[cfg(unix)]
fn bind_unix_socket(path_str: &str) -> Result<UnixSocketListener, IpcError> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::{UnixListener, UnixStream};

    let path = std::path::PathBuf::from(path_str);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if path.exists() {
        if UnixStream::connect(&path).is_err() {
            let _ = std::fs::remove_file(&path);
        } else {
            return Err(IpcError::Io {
                source: std::io::Error::new(
                    std::io::ErrorKind::AddrInUse,
                    format!("socket already in use: {path_str}"),
                ),
            });
        }
    }

    // Set restrictive umask (0o177) before UnixListener::bind creates the socket file,
    // and restore original umask immediately after (via RAII UmaskGuard), serialized by UMASK_LOCK.
    let listener = {
        let _lock = UMASK_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let _guard = UmaskGuard::new(0o177);
        let listener = UnixListener::bind(&path)?;
        let _ = listener.set_nonblocking(true);
        let perms = std::fs::Permissions::from_mode(0o600);
        let _ = std::fs::set_permissions(&path, perms);
        listener
    };

    Ok(UnixSocketListener {
        socket_path: path,
        listener,
    })
}

#[cfg(unix)]
fn connect_unix_socket(path_str: &str, timeout: Option<Duration>) -> Result<LocalStream, IpcError> {
    use std::os::unix::net::UnixStream;

    let path = std::path::Path::new(path_str);
    match UnixStream::connect(path) {
        Ok(stream) => {
            if let Some(t) = timeout {
                let _ = stream.set_read_timeout(Some(t));
                let _ = stream.set_write_timeout(Some(t));
            }
            Ok(LocalStream::new_unix(stream))
        }
        Err(err) => match err.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
                Err(IpcError::NotFound {
                    endpoint: path_str.to_owned(),
                })
            }
            std::io::ErrorKind::PermissionDenied => Err(IpcError::AccessDenied {
                endpoint: path_str.to_owned(),
            }),
            std::io::ErrorKind::TimedOut => Err(IpcError::Timeout {
                endpoint: path_str.to_owned(),
            }),
            _ => Err(IpcError::from(err)),
        },
    }
}

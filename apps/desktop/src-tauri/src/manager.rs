//! Spawn-or-attach manager coordinating discovery, detached spawning, and handshake negotiation (ADR 0006).

use std::sync::Arc;
use std::time::{Duration, Instant};

use altior_ipc::encode_frame;
use altior_protocol::{negotiate, CoreGreeting, CoreHello, EnvelopeLimits, NegotiatedHandshake};

use crate::adapter::{CoreChannel, CoreConnector};
use crate::discovery::CoreDiscovery;
use crate::error::BridgeError;
use crate::session::BridgeSession;
use crate::spawner::CoreSpawner;

/// Maximum time to wait for newly spawned Core process to initialize and publish its token file.
const SPAWN_ATTACH_TIMEOUT: Duration = Duration::from_millis(4000);
/// Interval between discovery polls when waiting for Core startup.
const POLL_INTERVAL: Duration = Duration::from_millis(60);

/// Manages attaching to an existing Core instance or detached-spawning a new one.
pub struct SpawnOrAttachManager {
    discovery: Arc<dyn CoreDiscovery>,
    spawner: Arc<dyn CoreSpawner>,
    connector: Arc<dyn CoreConnector>,
    session: Arc<BridgeSession>,
    limits: EnvelopeLimits,
}

impl SpawnOrAttachManager {
    /// Creates a new manager with explicit dependency injection.
    pub fn new(
        discovery: Arc<dyn CoreDiscovery>,
        spawner: Arc<dyn CoreSpawner>,
        connector: Arc<dyn CoreConnector>,
        session: Arc<BridgeSession>,
    ) -> Self {
        Self {
            discovery,
            spawner,
            connector,
            session,
            limits: EnvelopeLimits::default(),
        }
    }

    /// Primary entry point: attempts to attach to an active Core instance, or spawns Core detached.
    pub fn attach_or_spawn(
        &self,
    ) -> Result<(Box<dyn CoreChannel>, NegotiatedHandshake, CoreGreeting), BridgeError> {
        // Step 1: Probe existing discovery
        if let Ok(Some(creds)) = self.discovery.discover_credentials() {
            if let Ok(endpoint) = self.discovery.resolve_endpoint() {
                match self.connector.connect(&endpoint) {
                    Ok(channel) => {
                        match self.perform_handshake(&*channel, &creds.launch_token) {
                            Ok((negotiated, greeting)) => {
                                return Ok((channel, negotiated, greeting));
                            }
                            Err(_) => {
                                // Handshake failed or token was stale; invalidate discovery
                                let _ = self.discovery.invalidate_stale_token();
                            }
                        }
                    }
                    Err(_) => {
                        // Connection failed; endpoint is dead/stale
                        let _ = self.discovery.invalidate_stale_token();
                    }
                }
            }
        }

        // Step 2: Spawn new Core instance in detached mode
        let daemon_args = vec!["--daemon".to_string()];
        self.spawner
            .spawn_detached(&daemon_args)
            .map_err(|e| BridgeError::SpawnFailed(e.to_string()))?;

        // Step 3: Poll discovery and attach to the newly spawned Core
        let start = Instant::now();
        while start.elapsed() < SPAWN_ATTACH_TIMEOUT {
            std::thread::sleep(POLL_INTERVAL);

            let creds = match self.discovery.discover_credentials() {
                Ok(Some(creds)) => creds,
                _ => continue,
            };

            let endpoint = match self.discovery.resolve_endpoint() {
                Ok(ep) => ep,
                _ => continue,
            };

            let channel = match self.connector.connect(&endpoint) {
                Ok(ch) => ch,
                _ => continue,
            };

            match self.perform_handshake(&*channel, &creds.launch_token) {
                Ok((negotiated, greeting)) => {
                    return Ok((channel, negotiated, greeting));
                }
                Err(_) => {
                    continue;
                }
            }
        }

        Err(BridgeError::TransportUnavailable(
            "Timed out waiting for Core process to start, publish token, and accept handshake"
                .to_string(),
        ))
    }

    /// Performs the initial hello / greet exchange over an established channel.
    pub fn perform_handshake(
        &self,
        channel: &dyn CoreChannel,
        token: &altior_protocol::LaunchToken,
    ) -> Result<(NegotiatedHandshake, CoreGreeting), BridgeError> {
        let hello = self.session.create_hello(token);
        let hello_json =
            serde_json::to_string(&hello).map_err(|e| BridgeError::Serialization(e.to_string()))?;
        let hello_frame = encode_frame(&hello_json).map_err(BridgeError::from)?;

        channel
            .send_frame(&hello_frame)
            .map_err(BridgeError::from)?;

        // Read CoreHello response frame
        let core_hello_raw = channel
            .read_frame(Some(Duration::from_millis(3000)))
            .map_err(BridgeError::from)?;

        let core_hello: CoreHello = serde_json::from_slice(&core_hello_raw)
            .map_err(|e| BridgeError::HandshakeFailed(format!("Failed to parse CoreHello: {e}")))?;

        let negotiated = negotiate(&hello, &core_hello)
            .map_err(|e| BridgeError::HandshakeFailed(e.to_string()))?;

        // Read CoreGreeting response frame
        let greeting_raw = channel
            .read_frame(Some(Duration::from_millis(3000)))
            .map_err(BridgeError::from)?;

        let greeting: CoreGreeting = serde_json::from_slice(&greeting_raw).map_err(|e| {
            BridgeError::HandshakeFailed(format!("Failed to parse CoreGreeting: {e}"))
        })?;

        // Validate greeting epoch with session
        self.session.accept_greeting(&greeting, &negotiated)?;

        Ok((negotiated, greeting))
    }

    /// Returns a reference to the bridge session.
    #[must_use]
    pub fn session(&self) -> &Arc<BridgeSession> {
        &self.session
    }

    /// Returns envelope bounds limits.
    #[must_use]
    pub fn limits(&self) -> &EnvelopeLimits {
        &self.limits
    }
}

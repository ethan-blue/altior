//! Per-launch capability tokens (ADR 0006).
//!
//! Core mints one token per launch from OS entropy and publishes a token
//! file binding `{instance_id, token}`; Desktop reads that file and presents
//! the token in [`DesktopHello`]. This module owns the pure parts: minting
//! from **injected** entropy (tests pass fixed bytes; only the production
//! binary asks the OS), hex encoding, and the token-file JSON format. The
//! domain never generates identifiers or secrets (ADR 0004); this is
//! infrastructure, which is why it lives here.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use altior_domain::CoreInstanceId;
use altior_protocol::{LaunchToken, ProtocolError};

use crate::error::IpcError;

/// The bound pair published in the token file next to Core's endpoint.
#[derive(Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct LaunchCredentials {
    /// The Core launch the token authenticates.
    pub instance_id: CoreInstanceId,
    /// The opaque per-launch capability token.
    pub launch_token: LaunchToken,
}

impl fmt::Debug for LaunchCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LaunchCredentials")
            .field("instance_id", &self.instance_id)
            .field("launch_token", &"[REDACTED]")
            .finish()
    }
}

/// Mints a launch token from caller-supplied entropy.
///
/// `entropy` must carry at least 128 bits (16 bytes); it is hex-encoded into
/// a [`LaunchToken`]. Deterministic: the same entropy yields the same token,
/// which is exactly what tests need and what a real launch gets from one OS
/// draw.
///
/// # Errors
///
/// Returns [`IpcError::Protocol`] wrapping
/// [`ProtocolError::LaunchTokenLength`] when `entropy` is shorter than 16
/// bytes.
pub fn mint_launch_token(entropy: &[u8]) -> Result<LaunchToken, IpcError> {
    if entropy.len() < 16 {
        return Err(IpcError::Protocol {
            source: ProtocolError::LaunchTokenLength {
                length: entropy.len() * 2,
            },
        });
    }
    let mut hex = String::with_capacity(entropy.len() * 2);
    for byte in entropy {
        hex.push(HEX[(byte >> 4) as usize]);
        hex.push(HEX[(byte & 0x0F) as usize]);
    }
    LaunchToken::try_new(&hex).map_err(IpcError::from)
}

/// Encodes launch credentials as the canonical token-file JSON.
///
/// # Errors
///
/// Returns [`IpcError::Protocol`] when encoding fails.
pub fn encode_token_file(credentials: &LaunchCredentials) -> Result<String, IpcError> {
    serde_json::to_string(credentials).map_err(|error| IpcError::Protocol {
        source: ProtocolError::MalformedEnvelope { source: error },
    })
}

/// Parses launch credentials from token-file JSON.
///
/// # Errors
///
/// Returns [`IpcError::Protocol`] when the input is not a valid credentials
/// document.
pub fn decode_token_file(input: &str) -> Result<LaunchCredentials, IpcError> {
    serde_json::from_str(input).map_err(|error| IpcError::Protocol {
        source: ProtocolError::MalformedEnvelope { source: error },
    })
}

#[cfg(windows)]
fn get_os_entropy(buf: &mut [u8]) -> Result<(), std::io::Error> {
    #[link(name = "advapi32")]
    unsafe extern "system" {
        #[link_name = "SystemFunction036"]
        fn RtlGenRandom(RandomBuffer: *mut u8, RandomBufferLength: u32) -> u8;
    }
    let len = u32::try_from(buf.len())
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidInput, e))?;
    let ret = unsafe { RtlGenRandom(buf.as_mut_ptr(), len) };
    if ret != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(unix)]
fn get_os_entropy(buf: &mut [u8]) -> Result<(), std::io::Error> {
    use std::io::Read;
    let mut file = std::fs::File::open("/dev/urandom")?;
    file.read_exact(buf)?;
    Ok(())
}

/// Generates a cryptographically random launch token from OS entropy.
///
/// # Errors
/// Returns [`IpcError`] on OS RNG failure.
pub fn generate_launch_token() -> Result<LaunchToken, IpcError> {
    let mut entropy = [0u8; 16];
    get_os_entropy(&mut entropy)?;
    mint_launch_token(&entropy)
}

/// Generates a collision-safe random Core instance identifier with prefix `cor_`.
///
/// # Errors
/// Returns [`IpcError`] on OS RNG failure or ID validation error.
pub fn generate_instance_id() -> Result<CoreInstanceId, IpcError> {
    let mut entropy = [0u8; 16];
    get_os_entropy(&mut entropy)?;
    let mut hex = String::with_capacity(32);
    for byte in entropy {
        hex.push(HEX[(byte >> 4) as usize]);
        hex.push(HEX[(byte & 0x0F) as usize]);
    }
    CoreInstanceId::from_str(&format!("cor_{hex}")).map_err(|e| IpcError::InvalidEndpoint {
        reason: e.to_string(),
    })
}

/// Validates that a presented token matches the launch Core is running
/// using constant-time comparison to prevent timing attacks.
///
/// # Errors
///
/// Returns [`IpcError::AuthenticationRejected`] on any mismatch.
pub fn authenticate(presented: &LaunchToken, expected: &LaunchToken) -> Result<(), IpcError> {
    let p_bytes = presented.as_str().as_bytes();
    let e_bytes = expected.as_str().as_bytes();
    if constant_time_eq(p_bytes, e_bytes) {
        Ok(())
    } else {
        Err(IpcError::AuthenticationRejected)
    }
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

const HEX: [char; 16] = [
    '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'a', 'b', 'c', 'd', 'e', 'f',
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mints_deterministic_hex_tokens_from_injected_entropy() {
        let entropy = [
            0x0f, 0x1e, 0x2d, 0x3c, 0x4b, 0x5a, 0x69, 0x78, 0x87, 0x96, 0xa5, 0xb4, 0xc3, 0xd2,
            0xe1, 0xf0,
        ];
        let token = mint_launch_token(&entropy).unwrap();
        assert_eq!(token.as_str(), "0f1e2d3c4b5a69788796a5b4c3d2e1f0");
        // Same entropy, same token: determinism, not randomness.
        assert_eq!(mint_launch_token(&entropy).unwrap(), token);
    }

    #[test]
    fn rejects_entropy_below_128_bits() {
        assert!(matches!(
            mint_launch_token(&[0u8; 15]),
            Err(IpcError::Protocol { .. })
        ));
    }

    #[test]
    fn token_files_roundtrip() {
        let credentials = LaunchCredentials {
            instance_id: "cor_fixture000000009".parse().unwrap(),
            launch_token: mint_launch_token(&[0x11; 16]).unwrap(),
        };
        let encoded = encode_token_file(&credentials).unwrap();
        assert_eq!(
            encoded,
            r#"{"instance_id":"cor_fixture000000009","launch_token":"11111111111111111111111111111111"}"#
        );
        assert_eq!(decode_token_file(&encoded).unwrap(), credentials);
        assert!(decode_token_file("not json").is_err());
    }

    #[test]
    fn debug_redacts_launch_token() {
        let credentials = LaunchCredentials {
            instance_id: "cor_fixture000000009".parse().unwrap(),
            launch_token: mint_launch_token(&[0x11; 16]).unwrap(),
        };
        let debug_str = format!("{credentials:?}");
        assert!(!debug_str.contains("11111111111111111111111111111111"));
        assert!(debug_str.contains("[REDACTED]"));
    }

    #[test]
    fn generates_random_launch_tokens_and_instance_ids() {
        let token1 = generate_launch_token().unwrap();
        let token2 = generate_launch_token().unwrap();
        assert_ne!(token1, token2);
        assert_eq!(token1.as_str().len(), 32);

        let id1 = generate_instance_id().unwrap();
        let id2 = generate_instance_id().unwrap();
        assert_ne!(id1, id2);
        assert!(id1.as_str().starts_with("cor_"));
    }

    #[test]
    fn authentication_accepts_only_the_exact_token() {
        let expected = mint_launch_token(&[0x22; 16]).unwrap();
        authenticate(&expected, &expected).unwrap();
        let wrong = mint_launch_token(&[0x33; 16]).unwrap();
        assert!(matches!(
            authenticate(&wrong, &expected),
            Err(IpcError::AuthenticationRejected)
        ));
    }
}

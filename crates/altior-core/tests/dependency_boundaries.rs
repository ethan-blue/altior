//! Dependency-boundary acceptance evidence.
//!
//! Asserts the crate dependency direction required by AGENTS.md and
//! `docs/ARCHITECTURE.md`: `altior-domain` stays platform-neutral,
//! `altior-protocol` adds only serialization, `altior-ipc` adds only
//! contracts and JSON (ADR 0006), `altior-acp` is a replaceable harness
//! adapter beside the contracts (ADR 0007), and `altior-core` composes
//! the contract layers. The P0.5 spike crates sit outside the contract
//! core: `altior-storage` (ADR 0009), `altior-crdt` (ADR 0010),
//! `altior-crypto` (ADR 0011), and the zero-dependency `altior-relay`
//! (ADR 0012). The manifests are embedded at compile time, so the test
//! is deterministic and never reads the working tree at runtime.

const DOMAIN_MANIFEST: &str = include_str!("../../altior-domain/Cargo.toml");
const PROTOCOL_MANIFEST: &str = include_str!("../../altior-protocol/Cargo.toml");
const IPC_MANIFEST: &str = include_str!("../../altior-ipc/Cargo.toml");
const ACP_MANIFEST: &str = include_str!("../../altior-acp/Cargo.toml");
const CORE_MANIFEST: &str = include_str!("../Cargo.toml");
const STORAGE_MANIFEST: &str = include_str!("../../altior-storage/Cargo.toml");
const CRDT_MANIFEST: &str = include_str!("../../altior-crdt/Cargo.toml");
const CRYPTO_MANIFEST: &str = include_str!("../../altior-crypto/Cargo.toml");
const RELAY_MANIFEST: &str = include_str!("../../altior-relay/Cargo.toml");

/// Returns the dependency names declared in the `[dependencies]` section.
fn dependency_names(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let line = line.split('#').next().unwrap_or_default().trim();
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]";
            continue;
        }
        if in_dependencies
            && let Some(name) = line.split(['=', ' ']).next()
            && !name.is_empty()
        {
            names.push(name.to_owned());
        }
    }
    names
}

#[test]
fn domain_declares_no_platform_dependencies() {
    // serde is the only allowed dependency: a serialization library with
    // no platform coupling, recorded in ADR 0004. Anything else must be a
    // reviewable change to this boundary.
    assert_eq!(
        dependency_names(DOMAIN_MANIFEST),
        ["serde"],
        "altior-domain must stay platform-neutral"
    );
    for forbidden in ["tauri", "sqlite", "rusqlite", "acp", "tokio"] {
        assert!(
            !DOMAIN_MANIFEST.to_lowercase().contains(forbidden),
            "altior-domain manifest mentions {forbidden}"
        );
    }
}

#[test]
fn protocol_depends_only_on_domain_serialization_and_dto_codegen() {
    let names = dependency_names(PROTOCOL_MANIFEST);
    assert_eq!(
        names,
        ["altior-domain", "serde", "serde_json", "ts-rs"],
        "altior-protocol owns DTOs only; ts-rs is compile-time codegen (ADR 0005)"
    );
    for forbidden in ["tauri", "sqlite", "rusqlite", "acp", "tokio"] {
        assert!(
            !PROTOCOL_MANIFEST.to_lowercase().contains(forbidden),
            "altior-protocol manifest mentions {forbidden}"
        );
    }
}

#[test]
fn ipc_depends_only_on_domain_protocol_and_json() {
    // ADR 0006: the IPC layer is pure framing, endpoint derivation, auth,
    // and session machines — no async runtime, no OS transport crates.
    // The OS-facing slice (tokio named pipes / UDS) arrives as a later,
    // separately reviewed slice.
    assert_eq!(
        dependency_names(IPC_MANIFEST),
        ["altior-domain", "altior-protocol", "serde", "serde_json"],
        "altior-ipc must stay runtime- and OS-neutral (ADR 0006)"
    );
    for forbidden in ["tauri", "sqlite", "rusqlite", "acp", "tokio"] {
        assert!(
            !IPC_MANIFEST.to_lowercase().contains(forbidden),
            "altior-ipc manifest mentions {forbidden}"
        );
    }
}

#[test]
fn acp_depends_only_on_domain_protocol_and_json() {
    // ADR 0007: the adapter sits beside the contracts with the same
    // boundary as altior-ipc — no runtime, no OS transport, no ACP
    // client library adoption.
    assert_eq!(
        dependency_names(ACP_MANIFEST),
        ["altior-domain", "altior-protocol", "serde", "serde_json"],
        "altior-acp must stay runtime- and OS-neutral (ADR 0007)"
    );
    for forbidden in [
        "tauri",
        "sqlite",
        "rusqlite",
        "tokio",
        "agent-client-protocol",
    ] {
        assert!(
            !ACP_MANIFEST.to_lowercase().contains(forbidden),
            "altior-acp manifest mentions {forbidden}"
        );
    }
}

#[test]
fn core_depends_only_on_domain_ipc_and_protocol() {
    assert_eq!(
        dependency_names(CORE_MANIFEST),
        ["altior-domain", "altior-ipc", "altior-protocol"],
        "altior-core composes the contract layers; the ACP adapter wires \
         in behind the harness port in P1, not as a core dependency"
    );
}

#[test]
fn storage_depends_only_on_domain_protocol_and_rusqlite() {
    // ADR 0009: the persistence spike lives outside the contract
    // crates; SQLite is vendored via rusqlite's bundled feature.
    assert_eq!(
        dependency_names(STORAGE_MANIFEST),
        ["altior-domain", "altior-protocol", "rusqlite"],
        "altior-storage adds only the storage engine (ADR 0009)"
    );
}

#[test]
fn crdt_depends_only_on_the_two_raced_engines() {
    // ADR 0010: the bake-off crate implements the port twice and must
    // stay engine-only — no serialization, no contracts.
    assert_eq!(
        dependency_names(CRDT_MANIFEST),
        ["automerge", "loro"],
        "altior-crdt races exactly two engines behind the port (ADR 0010)"
    );
}

#[test]
fn crypto_depends_only_on_rustcrypto_and_dalek() {
    // ADR 0011: standard primitives only, all from the RustCrypto and
    // dalek lineages, no bespoke crypto.
    assert_eq!(
        dependency_names(CRYPTO_MANIFEST),
        [
            "chacha20poly1305",
            "ed25519-dalek",
            "hkdf",
            "sha2",
            "x25519-dalek",
            "zeroize"
        ],
        "altior-crypto is standard-library primitives (ADR 0011)"
    );
}

#[test]
fn relay_declares_no_runtime_dependencies() {
    // ADR 0012: the relay is a pure state machine — bytes, counters,
    // and a HashMap. Even altior-crypto is dev-only, for the
    // end-to-end integration test.
    assert!(
        dependency_names(RELAY_MANIFEST).is_empty(),
        "altior-relay must stay dependency-free (ADR 0012)"
    );
}

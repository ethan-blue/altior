//! P0.1 dependency-boundary acceptance evidence.
//!
//! Asserts the crate dependency direction required by AGENTS.md and
//! `docs/ARCHITECTURE.md`: `altior-domain` stays platform-neutral,
//! `altior-protocol` adds only serialization, and `altior-core` depends on
//! the two of them. The manifests are embedded at compile time, so the
//! test is deterministic and never reads the working tree at runtime.

const DOMAIN_MANIFEST: &str = include_str!("../../altior-domain/Cargo.toml");
const PROTOCOL_MANIFEST: &str = include_str!("../../altior-protocol/Cargo.toml");
const CORE_MANIFEST: &str = include_str!("../Cargo.toml");

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
        if in_dependencies {
            if let Some(name) = line.split(['=', ' ']).next() {
                if !name.is_empty() {
                    names.push(name.to_owned());
                }
            }
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
fn core_depends_only_on_domain_and_protocol() {
    let names = dependency_names(CORE_MANIFEST);
    assert_eq!(names, ["altior-domain", "altior-protocol"]);
}

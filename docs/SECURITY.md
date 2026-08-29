# Security model

## Protected assets

- personality and long-term memory
- conversation and provenance excerpts
- project knowledge and attachments
- Vault/device private keys
- provider credentials and agent auth state
- local project files and terminal authority

## Trust boundaries

- Desktop UI is less privileged than Core.
- Agent subprocesses and MCP servers are untrusted.
- Relay data is hostile until authenticated, decrypted, and schema-validated.
- A paired device may become compromised and must be revocable.
- Retrieved documents and memories are data, not trusted instructions.

## Required controls

- OS secret-store integration for private keys and provider secrets
- authenticated local IPC with per-launch capability token
- explicit project roots and path-containment checks
- command permissions bound to exact active turns
- encrypted, authenticated, replay-protected sync envelopes
- bounded message, document, attachment, and decompression sizes
- sensitive-value redaction before logs and crash reports
- device revocation, data-key rotation, and recovery-key export
- dependency audit, signed releases, and reproducible migration tests

## Prohibited shortcuts

- plaintext secrets in config files or environment snapshots
- relay-side decryption
- trusting filenames, MIME types, ACP metadata, or MCP schemas without validation
- shell command construction from concatenated untrusted strings
- automatic prompt resend after indeterminate delivery
- silent security fallback when keychain, signature, or permission checks fail

Before P3 implementation, expand this document into a reviewed threat model with
attack trees for pairing, relay compromise, stale devices, malicious agents, and
local privilege boundaries.


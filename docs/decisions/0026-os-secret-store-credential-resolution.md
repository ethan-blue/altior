# ADR 0026: OS Secret Store Credential Resolution and Platform Keychain Integration

- **Date:** 2026-09-13
- **Status:** Accepted
- **Scope:** `crates/altior-core`, `crates/altior-acp`, `docs/SECURITY.md`
- **Review Finding:** F35 / Task B02 (Production OS Secret Store replacement for `NoSecretsResolver` seam)

---

## 1. Context & Problem Statement

Altior core invariants (`AGENTS.md` "Sync and security") require:
1. **Device private keys and provider credentials live in the OS secret store.**
2. **Never persist credentials or private keys in SQLite, Markdown, logs, crash reports, IPC payload dumps, fixtures, or sync envelopes.**
3. **No custom cryptography**: Altior relies strictly on platform-native, reviewed primitives and standard system security facilities.

In P1.2 (ADR 0013 / ADR 0014), the ACP harness launch configuration introduced opaque secret references (`SecretRef`, `HarnessSecretRef`) and an integration seam (`SecretResolver` trait in `altior-acp`). However, the default production composition wired `NoSecretsResolver`, which intentionally failed closed with:
`"no secret resolver configured in composition"`.

As a result:
- When a user or agent binding specifies a credential reference (e.g. `vault:credentials:anthropic_key` or `sec_openai_01`), subprocess spawning immediately failed closed.
- Third-party model agents could not safely receive provider credentials (such as `ANTHROPIC_API_KEY`) from the operating system's native credential store.

---

## 2. Decision

### 1. Platform-Native OS Secret Store Driver in `altior-core`

Instead of introducing heavy, unreviewed third-party dependency trees, Altior Core implements a dedicated OS Secret Store driver behind the `SecretResolver` trait using platform-reviewed system facilities:

- **Windows (Implemented & Verified)**:
  - **Backend**: Windows Credential Management API via `windows-sys` (`CredReadW`, `CredWriteW`, `CredDeleteW`, `CREDENTIALW`, `CRED_TYPE_GENERIC` in `Advapi32.dll`).
  - **Namespace**: Credentials are targets namespaced under `Altior/credentials/<target_key>` or `Altior/<target_key>`.
  - **Persistence**: `CRED_PERSIST_LOCAL_MACHINE` for durable user-account scoped storage.
  - **Status**: Live round-trip tested on Windows host with guaranteed RAII cleanup.
- **macOS (Deferred / Fail-Closed)**:
  - **Backend**: Apple Keychain Services (`Security.framework` via `SecItemCopyMatching`, `SecItemAdd`, `SecItemDelete`).
  - **Status**: Implementation deferred to macOS developer host; currently cfg-gated and strictly fails closed (`SecretStoreError::NotFound`).
- **Linux (Deferred / Fail-Closed)**:
  - **Backend**: Freedesktop Secret Service API (`org.freedesktop.secrets` / libsecret) or user-session keyrings.
  - **Status**: Implementation deferred to Linux developer host; currently cfg-gated and strictly fails closed (`SecretStoreError::NotFound`).

### 2. SecretRef Canonical Mapping Rules

Secret references in Altior are opaque handles validated at construction. The OS Secret Store driver canonicalizes references into target keys:

1. `vault:credentials:<name>` -> maps to target `Altior/credentials/<name>`.
2. `secret://<service>/<account>` -> maps to target `Altior/<service>/<account>`.
3. `sec_<name>` -> maps to target `Altior/credentials/<name>`.
4. Plain labels `<name>` (e.g., `"anthropic_api_key"`) -> maps to target `Altior/credentials/<name>`.

Invalid characters (control chars, null bytes, characters exceeding 256 bytes) fail closed immediately at reference parsing.

### 3. Strict Privilege & Information Flow Boundaries

1. **Resolution Direction**: Credentials flow **strictly one-way**:
   $$\text{OS Secret Store} \xrightarrow{\text{Core Process Resolution}} \text{Child Process Environment}$$
2. **Desktop UI Isolation**:
   - The Desktop UI is unprivileged. Desktop commands NEVER request, receive, or inspect plaintext secret values.
   - IPC DTOs (`HarnessBindingConfigDto`, `HarnessBindingDto`) carry ONLY opaque strings (`HarnessSecretRef`).
   - Core provides management commands (store / delete / exists check) that take write payloads or delete keys, but NEVER echo stored secret plaintext back to Desktop.
3. **Database & Persistence Isolation**:
   - SQLite tables (`harness_binding`, `agent_profile`, `domain_journal`) persist ONLY `secret_refs_json`.
   - Any secret-shaped token attempted in SQLite content triggers `is_secret_shaped` fail-closed rejection.
4. **Diagnostic Redaction**:
   - `ResolvedLaunchConfig` implements custom `Debug` redaction (`"<redacted>"` for all environment variable values).
   - Core runtime diagnostics (`RuntimeDiagnosticsDto`) continuously scan and mask bearer tokens and key-value secret shapes.

### 4. Fail-Closed Error Model

When a secret cannot be resolved:
- The driver returns `AcpError::SecretResolutionFailed { secret_ref, diagnostic }`.
- The diagnostic string is sanitized: it contains the reference name and error category (e.g. `credential not found in Windows Credential Manager`), but **NEVER** plaintext material or environment dumps.
- Subprocess spawning is aborted immediately; no turn is marked active or indeterminate.

### 5. Testing & Fixture Seam

- **Production Core**: `AcpHarnessAdapter::new()` and `CoreApplication::new()` instantiate `OsSecretStore` as the production resolver.
- **Hermetic Unit & Composition Tests**: Retain `MockSecretResolver` / `NoSecretsResolver` for synthetic tests to prevent test suites from polluting or depending on developer system keychains.
- **Platform Verification Tests**: A dedicated integration test verifies real Windows Credential Manager write, read, and guaranteed cleanup using an ephemeral UUID target.

---

## 3. Migration & Exit Strategy

- **Format Migration**: Fully backward-compatible. Existing SQLite databases store `secret_refs_json` strings which remain valid references under the canonical namespace.
- **Exit Strategy**: Because the interface is isolated behind the `altior_acp::SecretResolver` trait object (`Arc<dyn SecretResolver + Send + Sync>`), replacing or extending the backend (e.g. swapping for a dedicated hardware enclave or TPM-backed provider in P3) requires zero changes to the domain, protocol, or SQLite schemas.

---

## 4. Consequences

### Positive
- Closes the P1 architectural gap identified in the 2026-09-13 technical review.
- Real third-party ACP agents can receive API keys securely without configuration file plaintext.
- Zero secret leakage into SQLite, Git, logs, IPC dumps, or sync envelopes.
- Complete fidelity to the `AGENTS.md` and `docs/AI_DEVELOPMENT.md` engineering mandates.

### Trade-offs
- Windows Credential Manager requires linking to `Advapi32.dll` via `windows-sys` and managing unsafe C FFI calls inside the Windows driver module.
- Non-Windows environments without a running Secret Service daemon or Keychain will fail closed on secret resolution until explicit credentials are provisioned in the OS store.

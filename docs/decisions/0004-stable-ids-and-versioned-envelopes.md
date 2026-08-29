# ADR 0004: Stable identifiers and versioned IPC envelopes

- Status: Accepted
- Date: 2026-08-29

## Context

P0.1 must freeze the contracts that `altior-core`, the Desktop client, and the
ACP adapter all depend on: stable entity identifiers, Desktop/Core protocol
version negotiation, capability representation, and the command/event envelope
shape. These decisions outlive any prompt or implementation slice: they become
part of persisted records and the IPC surface, so changing them later requires
migration.

Constraints from AGENTS.md and `docs/ARCHITECTURE.md`:

- `altior-domain` stays platform-neutral (no Tauri, ACP, SQLite, network, OS).
- Capability support is negotiated explicitly, never inferred from version
  strings.
- IPC payload sizes must be bounded.
- Unknown future events must not crash the process or leak into the stable
  schema; provider-native data may survive only as bounded diagnostics.
- Tests must be deterministic: no random identities, no wall-clock dependence.

## Decision

### Stable identifiers

1. Every first-class entity gets a distinct Rust newtype in `altior-domain`
   (`AgentProfileId`, `HarnessBindingId`, `ThreadId`, `TurnId`, `OperationId`,
   `EventId`). The types are structurally unrelated, so one kind of ID cannot
   be passed where another is expected.
2. The canonical string form is `<prefix>_<body>` where each type owns a fixed
   prefix (`agp_`, `hsb_`, `thr_`, `trn_`, `op_`, `evt_`) and the body is 16 to
   64 characters from `[0-9a-z]`. Total length stays under 128 bytes. The
   prefix makes cross-kind confusion fail at the string boundary as well as at
   the type boundary.
3. The domain defines parsing, validation, display, and serialization
   boundaries only. It does **not** generate identifiers: generation requires a
   randomness or UUID source, which is an infrastructure concern owned by
   `altior-core`. Tests and fixtures construct IDs by parsing fixed synthetic
   literals, which keeps them deterministic.
4. serde deserialization goes through the validator (`try_from = "String"`),
   so an invalid ID cannot enter the system through decoded data.

`serde` is added as a dependency of `altior-domain` for these derives. serde
is a data-encoding library, not a platform framework; it introduces no OS,
network, or database coupling. The domain types remain plain data.

Deferred IDs (`DeviceId`, `MemoryId`, `ProjectId`, `SkillId`, ...) are added
when their owning work package lands, not earlier. Adding a new ID type is
additive and requires no migration.

### Protocol version negotiation

1. The IPC protocol version is a plain positive integer newtype
   (`ProtocolVersion`). Each side advertises an inclusive
   `ProtocolVersionRange`, not a single number.
2. Negotiation intersects the two ranges and selects the **highest** common
   version. No intersection is a typed, explicit failure
   (`ProtocolError::NoCommonProtocolVersion`); there is no silent downgrade.
3. Product versions of Desktop and Core travel as a structured
   `ProductVersion` (`major.minor.patch`), used for diagnostics and upgrade
   prompts only. They never gate behavior; capabilities do that.

### Capabilities

1. A capability is a canonical identifier string (`[a-z0-9.-]`, at most 64
   bytes), e.g. `event.streaming`. Both sides declare a
   `CapabilitySet` mapping each known capability id to `supported` or
   `unsupported`.
2. A capability is negotiated only when **both** sides explicitly declare it
   supported. Capabilities claimed by only one side are recorded in
   `desktop_only` / `core_only` for diagnostics. Unknown (future) capability
   ids are data, not errors: they ride along and classify like any other id.
3. Capability ids are never derived from an agent or application version
   string.

### Command and event envelopes

1. Both envelopes are versioned: they carry the negotiated
   `protocol_version` and are validated against the locally supported range.
   A future envelope field may be added as an optional field; removing or
   resemanticing a field requires a new protocol version.
2. `CommandEnvelope`: `operation_id`, `kind` (a closed enum — unknown command
   kinds fail explicitly), an optional bounded JSON payload, and `issued_at`
   (`UnixMillis` supplied by the sender's clock; fixtures use constants).
3. `EventEnvelope`: `event_id`, optional `operation_id`/`thread_id`/`turn_id`,
   a `Sequence` (1-based; overflow is a typed error, never wraparound),
   `occurred_at`, and an `EventBody`.
4. `EventBody` has a small closed set of known normalized events plus one
   `Unknown` variant. An unrecognized wire event deserializes into `Unknown`
   carrying the provider kind name and a **bounded diagnostic** (the raw event
   object, capped). It never fails the connection and never enters stable
   domain records as structured data.
5. Envelope payloads are bounded at construction: text fields by their
   type-level byte cap, JSON payloads by an explicit `EnvelopeLimits`
   validation entry point. Oversized input is rejected with a typed error.
6. Envelope serialization is deterministic: struct field order is declaration
   order, and map keys inside payload values are sorted (serde_json's default
   `BTreeMap`-backed maps; the `preserve_order` feature stays off).

### Typed errors

`altior-domain` exposes `IdParseError`; `altior-protocol` exposes a
`#[non_exhaustive]` `ProtocolError`. Public fallible functions return these
types; no public API communicates failure through an ad-hoc string.

## Alternatives

- **UUID crate in `altior-domain`**: rejected for P0.1 — generation is not a
  domain concern and would drag a randomness dependency into the neutral core.
  Infrastructure may still mint UUIDv7 bodies later; the string contract above
  already accommodates that without change.
- **Unprefixed opaque strings**: one fewer rule, but a `ThreadId` accidentally
  stored in a `TurnId` column or field would parse fine; the prefix turns that
  class of bug into a validation failure.
- **Single equal version check (`mine == theirs`)**: simpler, but makes every
  compatible bump (Core adds v2 while still speaking v1) a hard failure and
  invites silent downgrade hacks. Range intersection with highest-common
  selection expresses real compatibility.
- **Semver ranges for the protocol**: rejected — protocol compatibility is
  defined by these envelopes, not by package semantics; an integer with an
  explicit negotiation rule is auditable.
- **Dropping unknown events on the floor**: loses diagnostics needed to debug
  provider drift; **passing them through raw** would let ACP/provider shapes
  leak into domain records. Bounded `Unknown` preservation is the middle
  ground required by `docs/HARNESSES.md`.
- **thiserror for error types**: convenient, but hand-written `Display` +
  `std::error::Error` impls keep the foundational crates dependency-lean.

## Failure modes

- **Cross-kind ID misuse**: prevented twice — distinct types (compile time)
  and per-type prefixes (validation time).
- **Downgrade attack / stale Core**: a version outside the local supported
  range is rejected with `UnsupportedProtocolVersion`; no silent fallback.
- **Oversized payloads**: rejected at construction and re-checked by
  `validate` against `EnvelopeLimits`.
- **Unbounded diagnostics from future providers**: `Unknown` diagnostics are
  capped; over-cap events fail that envelope with a typed error instead of
  ballooning memory.
- **Sequence wraparound**: `Sequence::next` returns `SequenceOverflow`; the
  value cannot silently restart.
- **Non-deterministic serialization**: field order is fixed and map keys are
  sorted; compatibility fixtures compare canonical forms.

## Migration path

- Adding a capability id, ID type, known event kind, or command kind is
  additive under the same protocol version (unknown values are preserved or
  explicitly rejected by design, so mixed versions degrade safely).
- Breaking envelope changes bump `ProtocolVersion`; both sides keep their
  advertised ranges until the older version is retired, then the retired
  version is removed from `SUPPORTED_PROTOCOL_VERSIONS` in a release note.
- The ID string format is append-only: new prefixes may be introduced, but
  existing prefixes and body rules never change for already-minted records.

## Exit strategy

All contract types are plain data with serde impls and no runtime coupling.
serde or serde_json can be swapped by reimplementing the `Serialize`/
`Deserialize` impls without touching domain behavior. The envelope layer can
be replaced wholesale if the P0.2 IPC transport ADR selects a schema-first
wire format, provided the same fixtures keep passing under the new codec.

## Revisit when

- The P0.2 transport ADR fixes the wire encoding and TypeScript DTO
  generation for Desktop.
- Real agent fixtures (P0.3) reveal normalized event kinds that the sample
  `KnownEvent` set must cover.
- Any persisted store starts minting IDs, at which point the generation
  convention (e.g. UUIDv7 bodies) is recorded here.

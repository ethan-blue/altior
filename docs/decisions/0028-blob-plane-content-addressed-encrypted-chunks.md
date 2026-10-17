# ADR 0028: Blob Plane — Content-Addressed Encrypted Chunks

- **Date:** 2026-10-17
- **Status:** Accepted (implements `docs/SYNC.md` plane 3 behind the ADR 0025
  gate; extends ADR 0027's engine with a fourth event family)
- **Scope:** `altior-crypto`, `altior-storage`, `altior-core::sync`

## 1. Context

`docs/SYNC.md` plane 3 carries opt-in attachments: content-addressed,
encrypted, chunked, and synchronized **outside** CRDT documents and outside
the journal's small-event path (AGENTS.md: "Large blobs are
content-addressed, chunked, encrypted, and synchronized outside CRDT
documents"). ADR 0027 covered the journal/document planes; this ADR is the
blob plane's format and sync contract.

Requirements inherited from the invariants:
- The relay never sees plaintext; blob bytes are opaque ciphertext.
- A forgotten/revoked device must not gain new read access (vault data key
  rotation covers blobs via re-encryption, out of this slice).
- Opt-in: no blob leaves a device unless its manifest event is created.
- Deterministic tests: no RNG in the library path.

## 2. Format

A blob is split into fixed **64 KiB plaintext chunks** (the final chunk is
the remainder). Per chunk:

1. `content_hash = SHA-256(plaintext)` — 32 bytes.
2. `chunk_key = HKDF-SHA256(ikm = vault_data_key, salt = content_hash,
   info = "altior/blob/chunk-key" ‖ version)` — **convergent** key: the
   same plaintext under the same vault derives the same key, so identical
   chunks produce identical ciphertext and identical storage identity,
   on every device, without coordination.
3. `nonce = HKDF-SHA256-expand(chunk_key, "altior/blob/nonce" ‖ chunk_index)`
   — deterministic per (content, index); AEAD nonce uniqueness holds
   because distinct plaintexts derive distinct keys, and one content's
   chunks have distinct indices. No RNG in the message path.
4. `ciphertext = ChaCha20-Poly1305(seal(chunk_key, nonce, plaintext,
   aad = content_hash ‖ chunk_index ‖ total_chunks ‖ blob_total_size))`.

The **blob id** is `SHA-256(ciphertext_0 ‖ ciphertext_1 ‖ …)` — the hash of
exactly the bytes at rest, so any store can verify a fetched chunk against
its own id and a reassembler can verify the whole. The **manifest** is the
ordered list `(content_hash, blob_id_per_chunk, chunk_len)` plus
`total_size`; it is small, and it travels as a signed journal event
(`EventBody::BlobManifest`), inheriting dedup, ordering, and the ack
frontier from the engine.

The vault data key comes from pairing (ADR 0027 §5); it never appears on
the wire and is not persisted by this slice (SecretStore, ADR 0026).

## 3. Sync semantics

- A device calls `put_blob(bytes)`: chunk, encrypt, store every chunk in
  its local CAS (`blob_chunks` table), and queue a `BlobManifest` journal
  event. Chunks do **not** ride the outbox — the manifest is the pointer.
- On applying a `BlobManifest`, a node records the manifest and marks the
  blob's chunks wanted; `flush`/`poll` exchange chunks opportunistically:
  a node pushes chunk envelopes (ciphertext ≤ 64 KiB + overhead, far below
  the relay's 1 MiB bound) for chunks its peers are missing, addressed by
  chunk blob id; receivers verify AEAD + digest before CAS insert. Chunks
  are idempotent (CAS insert or ignore).
- Reassembly (`get_blob`) requires every chunk present and verifies each
  chunk's AEAD tag and the whole-blob id; any miss fails closed with a
  typed error, never a partial plaintext.
- Deletion of an attachment is a journal-level concern (manifest
  retraction) and is deferred with the product-layer slices; chunk GC
  follows the same ack-frontier discipline as tombstones (ADR 0027 §6).

## 4. Alternatives considered

- **Random per-chunk keys** — stronger than convergent encryption against
  hash-collision confirmation attacks, but defeats cross-device dedup
  without a coordination round trip; the threat model (personal vault,
  attacker = curious relay) prefers dedup. Named for revisit if the
  threat model ever includes a global adversary.
- **Rolling-hash content-defined chunking** — better dedup under
  insertions, but non-deterministic across library versions and harder to
  test; fixed 64 KiB is a one-line revisit.
- **Blobs as journal events** — simple but abuses the 1 MiB relay bound
  and floods dedup; the manifest/chunk split keeps the journal small.

## 5. Failure modes

- Partial blob (missing chunks): `get_blob` fails closed; diagnostics list
  missing chunk ids.
- A hostile peer serving wrong bytes: AEAD tag and per-chunk digest
  both fail; the chunk is never stored.
- Vault key rotation: old blobs keep old keys until re-encryption
  (explicit product slice; revocation blocks new reads by the revoked
  device at the envelope layer regardless).

## 6. Migration / exit strategy

The `blob_chunks` table lands in storage v9 (unreleased, same branch).
The format is version-pinned (`version` byte in the chunk AAD context and
the manifest), so algorithm changes are new versions. Exit: drop the
table; journal manifests are inert events.

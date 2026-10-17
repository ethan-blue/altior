//! The Personal Vault sync engine (ADR 0027 §6, behind
//! [`crate::sync_gate::ensure_sync_allowed`]).
//!
//! Replicates a signed journal of events between paired devices over the
//! relay, every envelope sealed by a v2 session (epoch + counter lease +
//! turn ratchet). The three planes of `docs/SYNC.md` map onto this
//! module as:
//!
//! - **Knowledge journal**: [`JournalEvent`]s — Ed25519-signed by the
//!   originating device, deduplicated by event id, applied in relay
//!   order;
//! - **Knowledge documents**: [`EventBody::DocumentAppend`] — a
//!   grow-only entry set per document (state-based, commutative), which
//!   proves event-plane convergence; full SOUL/USER documents ride the
//!   Automerge engine (ADR 0010) and are out of this slice's scope;
//! - **Memory lifecycle**: propose/forget with permanent tombstones —
//!   a proposal for a tombstoned memory is suppressed and counted, the
//!   stale-device resurrection defense.
//!
//! Pull-before-push: [`SyncNode::flush`] polls every peer bucket first,
//! so a rejoining device applies the current tombstone frontier before
//! its own writes leave the building — the ADR 0025 threat-9 mitigation.
//!
//! Deliberately engine-local, test-scale simplifications (each named for
//! its production replacement): documents are grow-only sets rather than
//! Automerge; the outbox is pruned by nothing (production prunes below
//! the durable ack frontier); state lives in process memory (production
//! persists through `altior-storage` v9 tables). None of these weaken
//! the security properties the suites assert.

pub mod persist;

use std::collections::{BTreeMap, BTreeSet, HashMap};

use altior_crypto::{CounterLease, DeviceIdentity, DeviceKeys, SessionEpoch, SessionV2};
use sha2::{Digest, Sha256};

use altior_relay::{BucketId, Relay, RelayPolicy};
use persist::SyncStatePort;
use serde::{Deserialize, Serialize};

/// One replicated journal event: routing identity plus a signed body.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JournalEvent {
    /// Unique, origin-qualified: `"{origin}:{seq}"`.
    pub event_id: String,
    /// The originating device's label.
    pub origin: String,
    /// Monotonic per-origin sequence.
    pub origin_seq: u64,
    /// Ed25519 signature by the origin over the canonical body bytes.
    pub signature: [u8; 64],
    /// The event body.
    pub body: EventBody,
}

/// The replicated event vocabulary.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum EventBody {
    /// Appends an immutable entry to a document's grow-only set.
    DocumentAppend {
        /// The document's stable id.
        doc_id: String,
        /// The entry, unique within the document.
        entry: String,
    },
    /// Proposes a durable memory.
    MemoryProposed {
        /// The memory's stable id.
        memory_id: String,
        /// The memory content.
        content: String,
    },
    /// Forgets a memory permanently (ADR 0025 threat 10).
    MemoryForgotten {
        /// The forgotten memory's id.
        memory_id: String,
        /// When the forget was issued (Unix millis).
        forgotten_at_millis: u64,
        /// The tombstone digest binding this forget event.
        digest: [u8; 32],
    },
    /// This device's durable acknowledgement frontier: per event origin,
    /// the highest `origin_seq` it has applied. Acks flow as ordinary
    /// signed journal events, and the per-origin minimum across a node's
    /// peers gates tombstone compaction (ADR 0025 threat 10).
    Ack {
        /// Per event origin: the highest applied `origin_seq`.
        frontiers: BTreeMap<String, u64>,
    },
    /// The blob plane's manifest (ADR 0028): a signed pointer to a
    /// content-addressed encrypted blob. Chunks travel as blob
    /// envelopes, never as journal events.
    BlobAdded {
        /// The whole-blob identity: SHA-256 over the ordered chunk
        /// ciphertexts, hex-encoded.
        blob_id: String,
        /// Total plaintext size in bytes.
        total_size: u64,
        /// One entry per chunk, in order.
        entries: Vec<BlobEntry>,
    },
}

/// One chunk entry of a blob manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlobEntry {
    /// SHA-256 of the chunk plaintext, hex.
    pub content_hash: String,
    /// SHA-256 of the chunk ciphertext, hex (the CAS id).
    pub blob_id: String,
    /// The chunk ciphertext length in bytes.
    pub ciphertext_len: u64,
}

/// The wire payload of one v2 sync envelope: a signed journal event or
/// a blob-plane chunk (ADR 0028). Chunks are bulk data — they carry no
/// signature themselves; their authenticity is the signed manifest
/// plus the final AEAD verification at reassembly.
#[derive(Serialize, Deserialize)]
enum SyncPayload {
    Event(WireEvent),
    Chunk(BlobChunkWire),
}

/// A transported blob chunk (ADR 0028).
#[derive(Serialize, Deserialize)]
struct BlobChunkWire {
    blob_id: String,
    content_hash: String,
    chunk_id: String,
    ciphertext: Vec<u8>,
}

/// What went wrong in an engine operation.
#[derive(Debug)]
#[non_exhaustive]
pub enum SyncError {
    /// The event's origin is not a paired, known device.
    UnknownOrigin(String),
    /// The envelope failed to open, the signature failed, or a session
    /// operation was invalid.
    Session(altior_crypto::CryptoError),
    /// The relay refused a push.
    Relay(altior_relay::RelayError),
    /// Journal serialization failed.
    Serialization(serde_json::Error),
    /// An internal invariant broke (programming error, fail closed).
    Invariant(String),
    /// The durable state port failed; the node fails closed rather
    /// than continue with unrecorded state.
    State(std::io::Error),
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOrigin(origin) => write!(f, "event from unknown origin {origin}"),
            Self::Session(source) => write!(f, "session failure: {source}"),
            Self::Relay(source) => write!(f, "relay failure: {source}"),
            Self::Serialization(source) => write!(f, "journal serialization failed: {source}"),
            Self::Invariant(what) => write!(f, "sync invariant broken: {what}"),
            Self::State(source) => write!(f, "durable state failure: {source}"),
        }
    }
}

impl std::error::Error for SyncError {}

/// Lowercase hex encoding (blob plane ids).
fn hex_encode(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(bytes.len() * 2), |mut out, byte| {
            use std::fmt::Write as _;
            let _ = write!(out, "{byte:02x}");
            out
        })
}

/// Decodes 64 hex chars into a 32-byte id; `None` otherwise.
fn hex_decode(hex: &str) -> Option<[u8; 32]> {
    if hex.len() != 64 {
        return None;
    }
    let mut out = [0u8; 32];
    for (index, pair) in hex.as_bytes().chunks(2).enumerate() {
        out[index] = u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok()?;
    }
    Some(out)
}

/// The canonical signed bytes: exactly these fields, in this order.
#[derive(Serialize)]
struct Signable<'a> {
    event_id: &'a str,
    origin: &'a str,
    origin_seq: u64,
    body: &'a EventBody,
}

/// The wire form carried inside a v2 envelope.
#[derive(Serialize, Deserialize)]
struct WireEvent {
    event_id: String,
    origin: String,
    origin_seq: u64,
    signature: Vec<u8>,
    body: EventBody,
}

/// A known blob: its manifest plus whatever chunks are held locally.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct StoredBlob {
    total_size: u64,
    entries: Vec<BlobEntry>,
    /// Chunk ciphertext by chunk blob id (hex).
    chunks: BTreeMap<String, Vec<u8>>,
}

/// The tombstone mark recorded when a forget is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TombstoneMark {
    /// The tombstone digest.
    pub digest: [u8; 32],
    /// The forget event's origin (device label).
    pub origin: String,
    /// The forget event's per-origin sequence.
    pub origin_seq: u64,
}

/// One peer's channels: the two directional sessions and the receiver's
/// relay cursor into this peer's bucket.
struct PeerChannels {
    identity: DeviceIdentity,
    outbound: SessionV2,
    inbound: SessionV2,
    fetch_cursor: u64,
}

/// The durability mode behind a node: pure in-memory, or backed by a
/// [`SyncStatePort`] whose applied journal survives restarts.
enum SyncNodeState {
    Volatile,
    Durable(Box<dyn SyncStatePort>),
}

/// One paired device's sync engine state.
pub struct SyncNode {
    // Debug is manual: key material and session states are redacted.
    label: String,
    keys: DeviceKeys,
    peers: BTreeMap<String, PeerChannels>,
    /// The applied journal, by event id — also the deduplication set.
    applied: BTreeMap<String, JournalEvent>,
    /// Memory id -> content. Contradicted proposals never land here.
    memories: BTreeMap<String, String>,
    /// Memory id -> the mark binding the forget: digest plus the forget
    /// event's (origin, `origin_seq`) for the compaction gate.
    tombstones: BTreeMap<String, TombstoneMark>,
    /// Document id -> grow-only entry set.
    documents: BTreeMap<String, BTreeSet<String>>,
    /// Events created here, awaiting fan-out to peers.
    outbox: Vec<JournalEvent>,
    /// Per peer: that peer's reported applied frontier per event origin
    /// (from its Ack events). The per-origin minimum across peers is
    /// this node's tombstone compaction gate.
    peer_acks: BTreeMap<String, BTreeMap<String, u64>>,
    /// Fan-out bookkeeping: event ids already delivered to a peer.
    sent_to: HashMap<String, BTreeSet<String>>,
    /// Durability backing; `Volatile` behaves like the original engine.
    state: SyncNodeState,
    /// Fetch cursors loaded from the port at construction, applied
    /// when the peer's channel appears at `pair`.
    loaded_cursors: BTreeMap<String, u64>,
    /// The blob plane: known manifests and locally held chunks, keyed
    /// by whole-blob id (hex). In-memory for tests; durable CAS is
    /// storage's `blob_chunks` table.
    blobs: BTreeMap<String, StoredBlob>,
    next_seq: u64,
    suppressed_resurrections: u64,
}

impl std::fmt::Debug for SyncNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyncNode")
            .field("label", &self.label)
            .field("peers", &self.peers.keys().collect::<Vec<_>>())
            .field("applied", &self.applied.len())
            .field("memories", &self.memories.len())
            .field("tombstones", &self.tombstones.len())
            .field("documents", &self.documents.len())
            .field("outbox", &self.outbox.len())
            .field("suppressed_resurrections", &self.suppressed_resurrections)
            // keys, full peer sessions, sent_to and next_seq are
            // deliberately unsummarized; finish_non_exhaustive marks the
            // elision for the missing-fields lint.
            .finish_non_exhaustive()
    }
}

impl SyncNode {
    /// A node with no peers; pair it with [`pair`].
    #[must_use]
    pub fn new(keys: DeviceKeys, label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            keys,
            peers: BTreeMap::new(),
            applied: BTreeMap::new(),
            memories: BTreeMap::new(),
            tombstones: BTreeMap::new(),
            documents: BTreeMap::new(),
            peer_acks: BTreeMap::new(),
            outbox: Vec::new(),
            sent_to: HashMap::new(),
            next_seq: 1,
            suppressed_resurrections: 0,
            state: SyncNodeState::Volatile,
            loaded_cursors: BTreeMap::new(),
            blobs: BTreeMap::new(),
        }
    }

    /// A durable node: the port is loaded at construction and the
    /// applied journal is replayed through the normal apply path,
    /// rebuilding memories, documents, tombstones, and peer acks.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] when the port cannot be read;
    /// [`SyncError::Serialization`] when a stored body fails to decode.
    pub fn durable(
        keys: DeviceKeys,
        label: impl Into<String>,
        mut port: Box<dyn SyncStatePort>,
    ) -> Result<Self, SyncError> {
        let mut node = Self::new(keys, label);
        for event in port.load_applied()? {
            node.apply(event)?;
        }
        // The origin sequence must never rewind: event ids are unique
        // per device forever, so a restart continues past the highest
        // sequence in its own reloaded journal.
        node.next_seq = node
            .applied
            .values()
            .filter(|event| event.origin == node.label)
            .map(|event| event.origin_seq)
            .max()
            .unwrap_or(0)
            + 1;
        node.tombstones = port.load_tombstones()?;
        node.loaded_cursors = port.load_fetch_cursors()?;
        node.state = SyncNodeState::Durable(port);
        Ok(node)
    }

    /// Stores a blob locally: chunk, encrypt, and queue its manifest
    /// event for fan-out. Returns the whole-blob id (hex).
    ///
    /// # Errors
    ///
    /// [`SyncError::Session`] on encryption failure;
    /// [`SyncError::State`] in durable mode when the port cannot record.
    pub fn put_blob(&mut self, vault_key: &[u8; 32], bytes: &[u8]) -> Result<String, SyncError> {
        let (chunks, manifest) = altior_crypto::split_and_encrypt(vault_key, bytes);
        let manifest_event: Vec<BlobEntry> = manifest
            .entries
            .iter()
            .map(|entry| BlobEntry {
                content_hash: hex_encode(&entry.content_hash),
                blob_id: hex_encode(&entry.blob_id),
                ciphertext_len: entry.ciphertext_len as u64,
            })
            .collect();
        let blob_id = altior_crypto::blob_id_of(&manifest);
        let blob_id_hex = hex_encode(&blob_id);
        let stored = StoredBlob {
            total_size: manifest.total_size,
            entries: manifest_event.clone(),
            chunks: chunks
                .iter()
                .map(|chunk| (hex_encode(&chunk.blob_id), chunk.ciphertext.clone()))
                .collect(),
        };
        self.blobs.insert(blob_id_hex.clone(), stored);
        let event = self.create(EventBody::BlobAdded {
            blob_id: blob_id_hex.clone(),
            total_size: manifest.total_size,
            entries: manifest_event,
        });
        self.admit(event)?;
        Ok(blob_id_hex)
    }

    /// Reassembles a stored blob, verifying every chunk against the
    /// manifest before producing plaintext.
    ///
    /// # Errors
    ///
    /// [`SyncError::Session`] with [`altior_crypto::CryptoError::BlobChunkMissing`]
    /// while chunks are still inbound, or on any verification failure.
    pub fn get_blob(&self, vault_key: &[u8; 32], blob_id: &str) -> Result<Vec<u8>, SyncError> {
        let stored = self
            .blobs
            .get(blob_id)
            .ok_or_else(|| SyncError::Invariant(format!("unknown blob {blob_id}")))?;
        let manifest = altior_crypto::BlobManifest {
            total_size: stored.total_size,
            entries: stored
                .entries
                .iter()
                .filter_map(|entry| {
                    Some(altior_crypto::ChunkManifestEntry {
                        content_hash: hex_decode(&entry.content_hash)?,
                        blob_id: hex_decode(&entry.blob_id)?,
                        ciphertext_len: usize::try_from(entry.ciphertext_len).unwrap_or(0),
                    })
                })
                .collect(),
        };
        if manifest.entries.len() != stored.entries.len() {
            return Err(SyncError::Invariant("blob manifest hex decode".to_owned()));
        }
        let chunk_map: BTreeMap<[u8; 32], Vec<u8>> = stored
            .chunks
            .iter()
            .filter_map(|(id, data)| hex_decode(id).map(|key| (key, data.clone())))
            .collect();
        altior_crypto::decrypt_and_reassemble(vault_key, &manifest, &chunk_map)
            .map_err(SyncError::Session)
    }

    /// Records an event durably (durable mode) and applies it. The
    /// port's id dedup is authoritative across restarts.
    fn admit(&mut self, event: JournalEvent) -> Result<(), SyncError> {
        if self.applied.contains_key(&event.event_id) {
            return Ok(());
        }
        let fresh = match &mut self.state {
            SyncNodeState::Volatile => true,
            SyncNodeState::Durable(port) => port.record_applied(&event)?,
        };
        if fresh || !self.applied.contains_key(&event.event_id) {
            self.apply(event)?;
        }
        Ok(())
    }

    /// The node's device label.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Pairs `a` and `b`: identities exchanged, both directional v2
    /// sessions established under one epoch, each side drawing counters
    /// from its own durable lease.
    ///
    /// # Errors
    ///
    /// Session derivation failures ([`SyncError::Session`]).
    pub fn pair(
        a: &mut SyncNode,
        b: &mut SyncNode,
        epoch: &SessionEpoch,
        epoch_seq: u32,
        a_to_b: &mut dyn CounterLease,
        b_to_a: &mut dyn CounterLease,
    ) -> Result<(), SyncError> {
        let identity_a = a.keys.public_identity();
        let identity_b = b.keys.public_identity();
        let outbound_a =
            SessionV2::outbound(&a.keys, &identity_b, epoch.clone(), epoch_seq, a_to_b)
                .map_err(SyncError::Session)?;
        let inbound_b = SessionV2::inbound(&b.keys, &identity_a, epoch.clone(), epoch_seq)
            .map_err(SyncError::Session)?;
        let outbound_b =
            SessionV2::outbound(&b.keys, &identity_a, epoch.clone(), epoch_seq, b_to_a)
                .map_err(SyncError::Session)?;
        let inbound_a = SessionV2::inbound(&a.keys, &identity_b, epoch.clone(), epoch_seq)
            .map_err(SyncError::Session)?;

        let b_seed = a.loaded_cursors.get(&b.label).copied().unwrap_or(0);
        let a_seed = b.loaded_cursors.get(&a.label).copied().unwrap_or(0);
        a.peers.insert(
            b.label.clone(),
            PeerChannels {
                identity: identity_b,
                outbound: outbound_a,
                inbound: inbound_a,
                fetch_cursor: b_seed,
            },
        );
        b.peers.insert(
            a.label.clone(),
            PeerChannels {
                identity: identity_a,
                outbound: outbound_b,
                inbound: inbound_b,
                fetch_cursor: a_seed,
            },
        );
        Ok(())
    }

    /// Appends an entry to a document's grow-only set. Returns the
    /// event id.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] in durable mode when the port cannot record.
    pub fn append_document(&mut self, doc_id: &str, entry: &str) -> Result<String, SyncError> {
        let event = self.create(EventBody::DocumentAppend {
            doc_id: doc_id.to_owned(),
            entry: entry.to_owned(),
        });
        let id = event.event_id.clone();
        self.admit(event)?;
        Ok(id)
    }

    /// Proposes a memory. A proposal for a locally-tombstoned memory is
    /// refused at creation (the resurrection defense applies to the
    /// origin too).
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] in durable mode when the port cannot record.
    pub fn propose_memory(
        &mut self,
        memory_id: &str,
        content: &str,
    ) -> Result<Option<String>, SyncError> {
        if self.tombstones.contains_key(memory_id) {
            self.suppressed_resurrections += 1;
            return Ok(None);
        }
        let event = self.create(EventBody::MemoryProposed {
            memory_id: memory_id.to_owned(),
            content: content.to_owned(),
        });
        let id = event.event_id.clone();
        self.admit(event)?;
        Ok(Some(id))
    }

    /// Forgets a memory permanently and locally. Returns the event id.
    ///
    /// # Errors
    ///
    /// [`SyncError::State`] in durable mode when the port cannot record.
    pub fn forget_memory(
        &mut self,
        memory_id: &str,
        forgotten_at_millis: u64,
        digest: [u8; 32],
    ) -> Result<String, SyncError> {
        let event = self.create(EventBody::MemoryForgotten {
            memory_id: memory_id.to_owned(),
            forgotten_at_millis,
            digest,
        });
        let id = event.event_id.clone();
        self.admit(event)?;
        Ok(id)
    }

    /// Pulls every pending envelope from all peer buckets, verifies and
    /// applies them. Pull-before-push: this runs inside [`SyncNode::flush`]
    /// before any local write leaves the node.
    ///
    /// # Errors
    ///
    /// Envelope open, signature, or relay failures — fail closed, the
    /// node's applied state is unchanged for the rejected envelope.
    ///
    /// # Panics
    ///
    /// Panics if a peer's channel vanishes between listing and use —
    /// structurally impossible (single-threaded engine), so the panic
    /// marks a programming error.
    pub fn poll(&mut self, relay: &mut Relay) -> Result<usize, SyncError> {
        let mut applied_count = 0;
        let peer_labels: Vec<String> = self.peers.keys().cloned().collect();
        for peer in peer_labels {
            let bucket = BucketId::new(format!("vault/{}-from-{}", self.label, peer));
            loop {
                let page = relay.fetch(&bucket, self.peers[&peer].fetch_cursor, 256);
                let items = match page {
                    altior_relay::FetchPage::Entries {
                        items,
                        next_cursor,
                        has_more,
                    } => {
                        self.peers
                            .get_mut(&peer)
                            .expect("peer just read")
                            .fetch_cursor = next_cursor;
                        if let SyncNodeState::Durable(port) = &mut self.state {
                            port.record_fetch_cursor(&peer, next_cursor)?;
                        }
                        if items.is_empty() && !has_more {
                            break;
                        }
                        items
                    }
                    altior_relay::FetchPage::Compacted { .. }
                    | altior_relay::FetchPage::InvalidCursor { .. } => {
                        return Err(SyncError::Invariant(format!(
                            "unexpected relay page from {peer}"
                        )));
                    }
                };
                for item in items {
                    let plaintext = {
                        let channels = self.peers.get_mut(&peer).expect("peer just read");
                        match channels.inbound.open(&item.payload) {
                            Ok(plaintext) => plaintext,
                            // Pre-re-pair mail in the bucket: its epoch is
                            // older than the live pairing, its content was
                            // applied before the re-pair. Skip it; a NEWER
                            // epoch or an authentication failure fails the
                            // poll (fail closed).
                            Err(altior_crypto::CryptoError::EpochMismatch {
                                found_seq,
                                expected_seq,
                            }) if found_seq < expected_seq => {
                                continue;
                            }
                            Err(other) => return Err(SyncError::Session(other)),
                        }
                    };
                    // A payload is either a journal event or a blob chunk
                    // (ADR 0028). Chunks verify by digest, store into the
                    // CAS, and never touch the journal.
                    let wire: WireEvent = match serde_json::from_slice::<SyncPayload>(&plaintext)
                        .map_err(SyncError::Serialization)?
                    {
                        SyncPayload::Chunk(chunk) => {
                            self.ingest_chunk(peer.as_str(), chunk)?;
                            continue;
                        }
                        SyncPayload::Event(wire) => wire,
                    };
                    let origin = wire.origin.clone();
                    let identity = self
                        .peers
                        .get(&peer)
                        .map(|channels| channels.identity.clone())
                        .ok_or_else(|| SyncError::UnknownOrigin(origin.clone()))?;
                    if identity.id().as_str() != origin {
                        return Err(SyncError::UnknownOrigin(origin));
                    }
                    let signature: [u8; 64] = wire.signature.clone().try_into().map_err(|_| {
                        SyncError::Session(altior_crypto::CryptoError::SignatureInvalid)
                    })?;
                    DeviceKeys::verify_signature(
                        &identity,
                        &canonical_bytes(&wire.event_id, &wire.origin, wire.origin_seq, &wire.body),
                        &signature,
                    )
                    .map_err(SyncError::Session)?;

                    if self.applied.contains_key(&wire.event_id) {
                        continue;
                    }
                    let journal = JournalEvent {
                        event_id: wire.event_id,
                        origin: wire.origin,
                        origin_seq: wire.origin_seq,
                        signature,
                        body: wire.body,
                    };
                    self.admit(journal)?;
                    applied_count += 1;
                }
            }
        }
        Ok(applied_count)
    }

    /// Verifies and stores one inbound blob chunk (ADR 0028): the
    /// transport digest must match the claimed chunk id, and the chunk
    /// must belong to a known manifest. Final authenticity is the AEAD
    /// check at reassembly.
    fn ingest_chunk(&mut self, peer: &str, chunk: BlobChunkWire) -> Result<(), SyncError> {
        let digest: [u8; 32] = Sha256::digest(&chunk.ciphertext).into();
        if hex_encode(&digest) != chunk.chunk_id {
            return Err(SyncError::Invariant(format!(
                "chunk digest mismatch from {peer}"
            )));
        }
        if let Some(stored) = self.blobs.get_mut(&chunk.blob_id) {
            if !stored
                .entries
                .iter()
                .any(|entry| entry.blob_id == chunk.chunk_id)
            {
                return Err(SyncError::Invariant(format!(
                    "unsolicited chunk from {peer}"
                )));
            }
            stored
                .chunks
                .insert(chunk.chunk_id.clone(), chunk.ciphertext);
        }
        Ok(())
    }

    /// Blob chunk fan-out: every locally held chunk of every known
    /// blob, once per peer (ADR 0028 - chunks ride blob envelopes,
    /// not the journal).
    fn fan_out_blob_chunks(
        &mut self,
        relay: &mut Relay,
        peer_labels: &[String],
    ) -> Result<usize, SyncError> {
        let mut pushed = 0;
        let blob_ids: Vec<String> = self.blobs.keys().cloned().collect();
        for blob_id in blob_ids {
            let chunk_ids: Vec<String> = self.blobs[&blob_id].chunks.keys().cloned().collect();
            for chunk_id in chunk_ids {
                let dedup_key = format!("blob:{chunk_id}");
                for peer in peer_labels {
                    if self
                        .sent_to
                        .entry(dedup_key.clone())
                        .or_default()
                        .contains(peer)
                    {
                        continue;
                    }
                    let envelope = {
                        let stored = &self.blobs[&blob_id];
                        let entry = stored
                            .entries
                            .iter()
                            .find(|entry| entry.blob_id == chunk_id)
                            .expect("held chunk is in its manifest");
                        let payload = SyncPayload::Chunk(BlobChunkWire {
                            blob_id: blob_id.clone(),
                            content_hash: entry.content_hash.clone(),
                            chunk_id: chunk_id.clone(),
                            ciphertext: stored.chunks[&chunk_id].clone(),
                        });
                        let channels = self.peers.get_mut(peer).expect("peer listed");
                        channels
                            .outbound
                            .seal(&serde_json::to_vec(&payload).map_err(SyncError::Serialization)?)
                            .map_err(SyncError::Session)?
                    };
                    let bucket = BucketId::new(format!("vault/{peer}-from-{}", self.label));
                    relay
                        .push(
                            &bucket,
                            &format!("{}:blob:{}", self.label, chunk_id),
                            &envelope,
                        )
                        .map_err(SyncError::Relay)?;
                    self.sent_to
                        .entry(dedup_key.clone())
                        .or_default()
                        .insert(peer.clone());
                    pushed += 1;
                }
            }
        }
        Ok(pushed)
    }

    /// Pull-before-push: polls first, then fans every not-yet-sent
    /// outbox event out to every peer under one fresh ratchet turn.
    /// Returns the number of envelopes pushed.
    ///
    /// # Errors
    ///
    /// Poll failures, session, or relay failures.
    ///
    /// # Panics
    ///
    /// Panics if a peer's channel vanishes between listing and use —
    /// structurally impossible (single-threaded engine), so the panic
    /// marks a programming error.
    pub fn flush(&mut self, relay: &mut Relay, turn_seed: [u8; 32]) -> Result<usize, SyncError> {
        self.poll(relay)?;
        // Broadcast the durable acknowledgement frontier with every
        // batch, so peers can raise their compaction gates. Routed
        // through admit: the ack is journaled like any other event, or
        // a restarted node would rewind its origin sequence and collide
        // event ids on the relay.
        let ack = self.create(EventBody::Ack {
            frontiers: self.applied_frontier(),
        });
        self.admit(ack)?;
        let peer_labels: Vec<String> = self.peers.keys().cloned().collect();
        let mut pushed = 0;
        for peer in &peer_labels {
            // A turn IS a batch: peers with nothing pending keep their
            // turn position untouched, or the receiver would see a turn
            // skip and (correctly) reject the traffic as tampered.
            let has_unsent = self.outbox.iter().any(|event| {
                !self
                    .sent_to
                    .entry(event.event_id.clone())
                    .or_default()
                    .contains(peer)
            });
            if !has_unsent {
                continue;
            }
            // One ratchet turn per flush per direction.
            {
                let channels = self.peers.get_mut(peer).expect("peer listed");
                channels
                    .outbound
                    .open_turn(turn_seed)
                    .map_err(SyncError::Session)?;
            }
            for event in &self.outbox {
                if self
                    .sent_to
                    .entry(event.event_id.clone())
                    .or_default()
                    .contains(peer)
                {
                    continue;
                }
                let envelope = {
                    let channels = self.peers.get_mut(peer).expect("peer listed");
                    channels
                        .outbound
                        .seal(&serialize_event(event)?)
                        .map_err(SyncError::Session)?
                };
                let bucket = BucketId::new(format!("vault/{peer}-from-{}", self.label));
                relay
                    .push(
                        &bucket,
                        &format!("{}:{}", self.label, event.event_id),
                        &envelope,
                    )
                    .map_err(SyncError::Relay)?;
                self.sent_to
                    .get_mut(&event.event_id)
                    .expect("entry created above")
                    .insert(peer.clone());
                pushed += 1;
            }
        }
        pushed += self.fan_out_blob_chunks(relay, &peer_labels)?;
        Ok(pushed)
    }

    /// FNV-1a over the canonical state encoding: documents, memories,
    /// tombstones. Two nodes with equal digests have identical visible
    /// state.
    #[must_use]
    pub fn state_digest(&self) -> u64 {
        fn step(hash: &mut u64, bytes: &[u8]) {
            for byte in bytes {
                *hash ^= u64::from(*byte);
                *hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for (doc_id, entries) in &self.documents {
            step(&mut hash, doc_id.as_bytes());
            for entry in entries {
                step(&mut hash, entry.as_bytes());
                step(&mut hash, b"\x1e");
            }
            step(&mut hash, b"\x1d");
        }
        for (memory_id, content) in &self.memories {
            step(&mut hash, memory_id.as_bytes());
            step(&mut hash, content.as_bytes());
            step(&mut hash, b"\x1d");
        }
        for (memory_id, mark) in &self.tombstones {
            step(&mut hash, memory_id.as_bytes());
            step(&mut hash, &mark.digest);
            step(&mut hash, mark.origin.as_bytes());
            step(&mut hash, &mark.origin_seq.to_be_bytes());
            step(&mut hash, b"\x1d");
        }
        hash
    }

    /// The visible memory set.
    #[must_use]
    pub fn memories(&self) -> &BTreeMap<String, String> {
        &self.memories
    }

    /// The document entry sets.
    #[must_use]
    pub fn documents(&self) -> &BTreeMap<String, BTreeSet<String>> {
        &self.documents
    }

    /// The tombstone set.
    #[must_use]
    pub fn tombstones(&self) -> &BTreeMap<String, TombstoneMark> {
        &self.tombstones
    }

    /// This node's applied frontier: per event origin, the highest
    /// `origin_seq` present in the applied journal.
    #[must_use]
    pub fn applied_frontier(&self) -> BTreeMap<String, u64> {
        let mut frontier: BTreeMap<String, u64> = BTreeMap::new();
        for event in self.applied.values() {
            let entry = frontier.entry(event.origin.clone()).or_insert(0);
            if event.origin_seq > *entry {
                *entry = event.origin_seq;
            }
        }
        frontier
    }

    /// The threat-10 compaction gate: per event origin, the minimum
    /// frontier acknowledged by every direct peer. A tombstone from
    /// origin X at seq S may be reclaimed only when this map's entry
    /// for X is greater than S (every peer has applied the forget).
    /// Origins missing from the map are pinned (a peer never acked
    /// them — conservative).
    #[must_use]
    pub fn gc_frontiers(&self) -> BTreeMap<String, u64> {
        // Every known origin, then the minimum across ALL peers — a
        // peer with no ack ledger (never sent an Ack) contributes zero
        // for every origin and pins the gate, the same conservative
        // floor the domain AckFrontier models.
        let mut origins: BTreeSet<String> = BTreeSet::new();
        for acks in self.peer_acks.values() {
            origins.extend(acks.keys().cloned());
        }
        let empty = BTreeMap::new();
        let mut gate: BTreeMap<String, u64> = BTreeMap::new();
        for origin in origins {
            let mut floor = u64::MAX;
            for peer in self.peers.keys() {
                let acked = self
                    .peer_acks
                    .get(peer)
                    .unwrap_or(&empty)
                    .get(&origin)
                    .copied()
                    .unwrap_or(0);
                floor = floor.min(acked);
            }
            gate.insert(origin, floor);
        }
        gate
    }

    /// How many resurrection attempts (local or replicated) this node
    /// has suppressed — the suite-4 diagnostic.
    #[must_use]
    pub fn suppressed_resurrections(&self) -> u64 {
        self.suppressed_resurrections
    }

    /// How many journal events this node has applied.
    #[must_use]
    pub fn applied_count(&self) -> usize {
        self.applied.len()
    }

    /// Creates, signs, and outboxes a local event. Local application is
    /// the caller's next step.
    fn create(&mut self, body: EventBody) -> JournalEvent {
        let origin_seq = self.next_seq;
        self.next_seq += 1;
        let event_id = format!("{}:{origin_seq}", self.label);
        let signature = self
            .keys
            .sign(&canonical_bytes(&event_id, &self.label, origin_seq, &body));
        let event = JournalEvent {
            event_id,
            origin: self.label.clone(),
            origin_seq,
            signature,
            body,
        };
        self.outbox.push(event.clone());
        event
    }

    /// Applies an event to visible state, enforcing the tombstone
    /// contract (threat 10): a proposal for a forgotten memory is
    /// suppressed and counted, never applied.
    fn apply(&mut self, event: JournalEvent) -> Result<(), SyncError> {
        match &event.body {
            EventBody::DocumentAppend { doc_id, entry } => {
                self.documents
                    .entry(doc_id.clone())
                    .or_default()
                    .insert(entry.clone());
            }
            EventBody::MemoryProposed { memory_id, content } => {
                if self.tombstones.contains_key(memory_id) {
                    self.suppressed_resurrections += 1;
                    self.applied.insert(event.event_id.clone(), event);
                    return Ok(());
                }
                self.memories.insert(memory_id.clone(), content.clone());
            }
            EventBody::MemoryForgotten {
                memory_id,
                digest,
                forgotten_at_millis,
                ..
            } => {
                let mark = TombstoneMark {
                    digest: *digest,
                    origin: event.origin.clone(),
                    origin_seq: event.origin_seq,
                };
                if let SyncNodeState::Durable(port) = &mut self.state {
                    port.record_tombstone(memory_id, &mark, *forgotten_at_millis)
                        .map_err(|error| match error {
                            SyncError::State(source) => SyncError::State(source),
                            other => SyncError::Invariant(other.to_string()),
                        })?;
                }
                self.tombstones.insert(memory_id.clone(), mark);
                self.memories.remove(memory_id);
            }
            EventBody::BlobAdded {
                blob_id,
                total_size,
                entries,
            } => {
                let stored = self.blobs.entry(blob_id.clone()).or_default();
                stored.total_size = *total_size;
                stored.entries.clone_from(entries);
                self.applied.insert(event.event_id.clone(), event);
                return Ok(());
            }
            EventBody::Ack { frontiers } => {
                let ledger = self.peer_acks.entry(event.origin.clone()).or_default();
                for (origin, seq) in frontiers {
                    let entry = ledger.entry(origin.clone()).or_insert(0);
                    if *seq > *entry {
                        *entry = *seq;
                    }
                }
            }
        }
        self.applied.insert(event.event_id.clone(), event);
        Ok(())
    }
}

/// The bytes a signature covers: canonical JSON of the routing fields
/// and body (struct order, no maps — deterministic).
#[allow(clippy::needless_pass_by_value)]
fn canonical_bytes(event_id: &str, origin: &str, origin_seq: u64, body: &EventBody) -> Vec<u8> {
    serde_json::to_vec(&Signable {
        event_id,
        origin,
        origin_seq,
        body,
    })
    .expect("canonical serialization of owned data cannot fail")
}

fn serialize_event(event: &JournalEvent) -> Result<Vec<u8>, SyncError> {
    serde_json::to_vec(&SyncPayload::Event(WireEvent {
        event_id: event.event_id.clone(),
        origin: event.origin.clone(),
        origin_seq: event.origin_seq,
        signature: event.signature.to_vec(),
        body: event.body.clone(),
    }))
    .map_err(SyncError::Serialization)
}

/// Relay policy for vault traffic: the permissive test preset. Production
/// deployments use [`RelayPolicy::production`].
#[must_use]
pub fn vault_relay() -> Relay {
    Relay::new(RelayPolicy::permissive())
}

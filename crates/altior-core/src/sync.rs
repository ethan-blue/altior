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

use std::collections::{BTreeMap, BTreeSet, HashMap};

use altior_crypto::{CounterLease, DeviceIdentity, DeviceKeys, SessionEpoch, SessionV2};
use altior_relay::{BucketId, Relay, RelayPolicy};
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
}

impl std::fmt::Display for SyncError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownOrigin(origin) => write!(f, "event from unknown origin {origin}"),
            Self::Session(source) => write!(f, "session failure: {source}"),
            Self::Relay(source) => write!(f, "relay failure: {source}"),
            Self::Serialization(source) => write!(f, "journal serialization failed: {source}"),
            Self::Invariant(what) => write!(f, "sync invariant broken: {what}"),
        }
    }
}

impl std::error::Error for SyncError {}

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

/// One peer's channels: the two directional sessions and the receiver's
/// relay cursor into this peer's bucket.
struct PeerChannels {
    identity: DeviceIdentity,
    outbound: SessionV2,
    inbound: SessionV2,
    fetch_cursor: u64,
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
    /// Memory id -> tombstone digest. Permanent.
    tombstones: BTreeMap<String, [u8; 32]>,
    /// Document id -> grow-only entry set.
    documents: BTreeMap<String, BTreeSet<String>>,
    /// Events created here, awaiting fan-out to peers.
    outbox: Vec<JournalEvent>,
    /// Fan-out bookkeeping: event ids already delivered to a peer.
    sent_to: HashMap<String, BTreeSet<String>>,
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
            outbox: Vec::new(),
            sent_to: HashMap::new(),
            next_seq: 1,
            suppressed_resurrections: 0,
        }
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
        a_to_b: &mut dyn CounterLease,
        b_to_a: &mut dyn CounterLease,
    ) -> Result<(), SyncError> {
        let identity_a = a.keys.public_identity();
        let identity_b = b.keys.public_identity();
        let outbound_a = SessionV2::outbound(&a.keys, &identity_b, epoch.clone(), 1, a_to_b)
            .map_err(SyncError::Session)?;
        let inbound_b = SessionV2::inbound(&b.keys, &identity_a, epoch.clone(), 1)
            .map_err(SyncError::Session)?;
        let outbound_b = SessionV2::outbound(&b.keys, &identity_a, epoch.clone(), 1, b_to_a)
            .map_err(SyncError::Session)?;
        let inbound_a = SessionV2::inbound(&a.keys, &identity_b, epoch.clone(), 1)
            .map_err(SyncError::Session)?;

        a.peers.insert(
            b.label.clone(),
            PeerChannels {
                identity: identity_b,
                outbound: outbound_a,
                inbound: inbound_a,
                fetch_cursor: 0,
            },
        );
        b.peers.insert(
            a.label.clone(),
            PeerChannels {
                identity: identity_a,
                outbound: outbound_b,
                inbound: inbound_b,
                fetch_cursor: 0,
            },
        );
        Ok(())
    }

    /// Appends an entry to a document's grow-only set. Returns the
    /// event id.
    pub fn append_document(&mut self, doc_id: &str, entry: &str) -> String {
        let event = self.create(EventBody::DocumentAppend {
            doc_id: doc_id.to_owned(),
            entry: entry.to_owned(),
        });
        let id = event.event_id.clone();
        self.apply(event);
        id
    }

    /// Proposes a memory. A proposal for a locally-tombstoned memory is
    /// refused at creation (the resurrection defense applies to the
    /// origin too).
    pub fn propose_memory(&mut self, memory_id: &str, content: &str) -> Option<String> {
        if self.tombstones.contains_key(memory_id) {
            self.suppressed_resurrections += 1;
            return None;
        }
        let event = self.create(EventBody::MemoryProposed {
            memory_id: memory_id.to_owned(),
            content: content.to_owned(),
        });
        let id = event.event_id.clone();
        self.apply(event);
        Some(id)
    }

    /// Forgets a memory permanently and locally.
    pub fn forget_memory(&mut self, memory_id: &str, forgotten_at_millis: u64, digest: [u8; 32]) {
        let event = self.create(EventBody::MemoryForgotten {
            memory_id: memory_id.to_owned(),
            forgotten_at_millis,
            digest,
        });
        self.apply(event);
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
                        channels
                            .inbound
                            .open(&item.payload)
                            .map_err(SyncError::Session)?
                    };
                    let wire: WireEvent =
                        serde_json::from_slice(&plaintext).map_err(SyncError::Serialization)?;
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
                    self.apply(journal);
                    applied_count += 1;
                }
            }
        }
        Ok(applied_count)
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
        for (memory_id, digest) in &self.tombstones {
            step(&mut hash, memory_id.as_bytes());
            step(&mut hash, digest);
            step(&mut hash, b"\x1d");
        }
        hash
    }

    /// The visible memory set.
    #[must_use]
    pub fn memories(&self) -> &BTreeMap<String, String> {
        &self.memories
    }

    /// The tombstone set.
    #[must_use]
    pub fn tombstones(&self) -> &BTreeMap<String, [u8; 32]> {
        &self.tombstones
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
    fn apply(&mut self, event: JournalEvent) {
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
                    return;
                }
                self.memories.insert(memory_id.clone(), content.clone());
            }
            EventBody::MemoryForgotten { memory_id, .. } => {
                self.tombstones.insert(memory_id.clone(), {
                    match &event.body {
                        EventBody::MemoryForgotten { digest, .. } => *digest,
                        _ => unreachable!("matched arm"),
                    }
                });
                self.memories.remove(memory_id);
            }
        }
        self.applied.insert(event.event_id.clone(), event);
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
    serde_json::to_vec(&WireEvent {
        event_id: event.event_id.clone(),
        origin: event.origin.clone(),
        origin_seq: event.origin_seq,
        signature: event.signature.to_vec(),
        body: event.body.clone(),
    })
    .map_err(SyncError::Serialization)
}

/// Relay policy for vault traffic: the permissive test preset. Production
/// deployments use [`RelayPolicy::production`].
#[must_use]
pub fn vault_relay() -> Relay {
    Relay::new(RelayPolicy::permissive())
}

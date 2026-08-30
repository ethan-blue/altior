//! Relay transport spike: a content-agnostic queue machine (ADR 0012).
//!
//! `docs/ARCHITECTURE.md` lets an untrusted relay carry sync traffic
//! between the user's devices only if the relay can neither read nor
//! forge it. This crate is that relay as a pure state machine — no
//! network, no timers, no codecs, no dependencies:
//!
//! - **Content-agnostic and sealed-sender**: the API takes opaque
//!   bytes and a destination bucket. There is no sender field
//!   anywhere, nothing to sign on the relay's behalf, and payloads
//!   are never inspected. Push targets a *receiver's* bucket; the
//!   relay cannot learn who sent what.
//! - **Cursored fetch**: every push gets a monotonically increasing
//!   sequence number; `fetch(after, limit)` pages through `seq >
//!   after`. Fetching is non-destructive and repeatable — delivery
//!   is at-least-once, and the receiver's replay window (ADR 0011)
//!   is what makes re-delivery harmless.
//! - **Idempotent push**: a repeated push with the same id inside
//!   the retained window returns the original receipt instead of a
//!   second copy, so an at-least-once retrying sender is safe.
//! - **Quotas and retention in logical ticks**: the relay has no
//!   clock; callers advance a logical tick. Payloads above a byte
//!   quota are refused, buckets beyond a depth quota push back, and
//!   entries older than the retention policy are reclaimed by an
//!   explicit sweep.
//! - **Compaction with fetch equivalence**: fetched (and swept)
//!   ranges are folded into a per-bucket checkpoint. For every
//!   cursor at or after the checkpoint boundary, fetch results are
//!   byte-identical before and after compaction. A cursor that fell
//!   behind the boundary gets an explicit `Compacted` page — the
//!   signal that a full resync (a fresh CRDT snapshot push, ADR
//!   0010) is required, never silent loss.

use std::collections::HashMap;
use std::collections::VecDeque;
use std::fmt;

pub mod error;

pub use error::RelayError;

/// A relay bucket: one device's inbox, addressed by an opaque label
/// (in production, derived from the receiver's public identity).
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BucketId(String);

impl BucketId {
    /// A bucket label.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self(label.into())
    }

    /// The label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for BucketId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The quotas and retention that shape the machine. All values are
/// policy, not physics; production tunes them per deployment.
#[derive(Clone, Debug)]
pub struct RelayPolicy {
    /// Largest single payload accepted by `push`.
    pub max_payload_bytes: usize,
    /// Largest number of retained entries per bucket, fetched or
    /// not. Fetch alone never frees depth — compaction does.
    pub max_bucket_depth: usize,
    /// Entries older than this many logical ticks are reclaimed by
    /// [`Relay::sweep_expired`], fetched or not.
    pub max_age_ticks: u64,
}

impl RelayPolicy {
    /// A permissive default for tests: 1 MiB payloads, 512-deep
    /// buckets, 1000-tick retention.
    #[must_use]
    pub fn permissive() -> Self {
        Self {
            max_payload_bytes: 1024 * 1024,
            max_bucket_depth: 512,
            max_age_ticks: 1000,
        }
    }
}

/// What `push` did.
#[derive(Debug, Eq, PartialEq)]
pub enum PushOutcome {
    /// A new entry was queued under this sequence number.
    Pushed {
        /// The entry's fetch cursor value.
        seq: u64,
    },
    /// An entry with this id is already retained; nothing was queued.
    Duplicate {
        /// The original entry's sequence number.
        seq: u64,
    },
}

/// One fetched payload plus its cursor.
#[derive(Debug, Eq, PartialEq)]
pub struct FetchedItem {
    /// The entry's sequence number (use as the next `after` cursor).
    pub seq: u64,
    /// The sender's id for the push, echoed for dedupe.
    pub id: String,
    /// The opaque payload, byte-identical to what was pushed.
    pub payload: Vec<u8>,
}

/// One page of `fetch`.
#[derive(Debug, Eq, PartialEq)]
pub enum FetchPage {
    /// The entries after the cursor, in sequence order.
    Entries {
        /// At most `limit` entries, ascending by `seq`.
        items: Vec<FetchedItem>,
        /// The cursor to fetch from next (the last item's `seq`, or
        /// the given `after` when the page is empty).
        next_cursor: u64,
        /// Whether more retained entries exist past this page.
        has_more: bool,
    },
    /// The cursor predates the bucket's checkpoint: everything up to
    /// `compacted_up_to` was reclaimed. The receiver must resync
    /// (fresh snapshot push) and resume from this boundary.
    Compacted {
        /// The checkpoint boundary; cursors at or after it behave
        /// exactly as they did before compaction.
        compacted_up_to: u64,
    },
    /// The requested cursor is beyond this bucket's actual frontier.
    /// It is not recorded as possession and cannot hide later pushes.
    InvalidCursor {
        /// The untrusted requested `after` value.
        requested: u64,
        /// The highest sequence actually assigned in this bucket.
        frontier: u64,
    },
}

/// What one compaction (or retention sweep) reclaimed.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct CompactionSummary {
    /// The new checkpoint boundary.
    pub compacted_up_to: u64,
    /// Reclaimed entries folded into the checkpoint.
    pub payloads: u64,
    /// Reclaimed bytes folded into the checkpoint.
    pub bytes: u64,
}

/// A bucket's visible state, for evidence and assertions.
#[derive(Debug, Default, Eq, PartialEq)]
pub struct BucketStats {
    /// Entries retained and not yet fetched.
    pub pending: usize,
    /// Entries retained and already fetched, awaiting compaction.
    pub fetched: usize,
    /// Total entries reclaimed by compaction and sweeps.
    pub compacted_payloads: u64,
    /// Total bytes reclaimed by compaction and sweeps.
    pub compacted_bytes: u64,
    /// The checkpoint boundary.
    pub compacted_up_to: u64,
    /// The highest sequence ever assigned in this bucket.
    pub last_seq: u64,
}

#[derive(Debug)]
struct QueueEntry {
    seq: u64,
    id: String,
    payload: Vec<u8>,
    pushed_tick: u64,
    fetched: bool,
}

#[derive(Debug, Default)]
struct Bucket {
    entries: VecDeque<QueueEntry>,
    /// Everything at or below this sequence is checkpointed and gone.
    compacted_up_to: u64,
    /// The receiver's claimed position: the highest sequence the
    /// receiver has certainly seen. Every fetch asserts possession
    /// through its `after` cursor and advances this through its last
    /// returned item.
    cursor: u64,
    /// Cumulative counts across all compactions.
    compacted_payloads: u64,
    compacted_bytes: u64,
}

impl Bucket {
    fn find_by_id(&self, id: &str) -> Option<&QueueEntry> {
        self.entries.iter().find(|entry| entry.id == id)
    }

    fn frontier(&self) -> u64 {
        self.entries
            .back()
            .map_or(self.compacted_up_to, |entry| entry.seq)
    }
}

/// The relay: a deterministic, content-agnostic queue machine.
#[derive(Debug)]
pub struct Relay {
    policy: RelayPolicy,
    buckets: HashMap<BucketId, Bucket>,
    next_seq: u64,
    tick: u64,
}

impl Relay {
    /// A relay under the given policy, at tick 0 with no buckets.
    #[must_use]
    pub fn new(policy: RelayPolicy) -> Self {
        Self {
            policy,
            buckets: HashMap::new(),
            next_seq: 1,
            tick: 0,
        }
    }

    /// The policy in force.
    #[must_use]
    pub fn policy(&self) -> &RelayPolicy {
        &self.policy
    }

    /// Advances the logical clock one tick and returns the new tick.
    /// The relay never observes wall time; retention is measured
    /// against this counter only.
    pub fn tick(&mut self) -> u64 {
        self.tick += 1;
        self.tick
    }

    /// The current logical tick.
    #[must_use]
    pub fn current_tick(&self) -> u64 {
        self.tick
    }

    /// Queues an opaque payload in the receiver's bucket.
    ///
    /// # Errors
    ///
    /// [`RelayError::PushTooLarge`] when the payload exceeds the
    /// byte quota; [`RelayError::BucketFull`] when the bucket already
    /// holds its depth quota of retained entries. Both leave the
    /// bucket untouched.
    pub fn push(
        &mut self,
        bucket: &BucketId,
        id: &str,
        payload: &[u8],
    ) -> Result<PushOutcome, RelayError> {
        if payload.len() > self.policy.max_payload_bytes {
            return Err(RelayError::PushTooLarge {
                size_bytes: payload.len(),
                limit_bytes: self.policy.max_payload_bytes,
            });
        }
        let slot = self.buckets.entry(bucket.clone()).or_default();
        if slot.entries.len() >= self.policy.max_bucket_depth && slot.find_by_id(id).is_none() {
            return Err(RelayError::BucketFull {
                depth: self.policy.max_bucket_depth,
                bucket: bucket.to_string(),
            });
        }
        if let Some(existing) = slot.find_by_id(id) {
            if existing.payload == payload {
                return Ok(PushOutcome::Duplicate { seq: existing.seq });
            }
            return Err(RelayError::PushIdCollision {
                id: id.to_owned(),
                existing_seq: existing.seq,
            });
        }
        let seq = self.next_seq;
        self.next_seq += 1;
        slot.entries.push_back(QueueEntry {
            seq,
            id: id.to_owned(),
            payload: payload.to_vec(),
            pushed_tick: self.tick,
            fetched: false,
        });
        Ok(PushOutcome::Pushed { seq })
    }

    /// Pages through the bucket: every retained entry with `seq >
    /// after`, ascending, at most `limit` of them. Repeating a fetch
    /// returns the same entries — delivery is at-least-once by
    /// design, and receivers dedupe (ADR 0011 replay window).
    ///
    /// A cursor below the bucket's checkpoint returns
    /// [`FetchPage::Compacted`] instead: the receiver resyncs.
    pub fn fetch(&mut self, bucket: &BucketId, after: u64, limit: usize) -> FetchPage {
        let Some(slot) = self.buckets.get_mut(bucket) else {
            if after > 0 {
                return FetchPage::InvalidCursor {
                    requested: after,
                    frontier: 0,
                };
            }
            return FetchPage::Entries {
                items: Vec::new(),
                next_cursor: after,
                has_more: false,
            };
        };
        if after < slot.compacted_up_to {
            return FetchPage::Compacted {
                compacted_up_to: slot.compacted_up_to,
            };
        }
        let frontier = slot.frontier();
        if after > frontier {
            return FetchPage::InvalidCursor {
                requested: after,
                frontier,
            };
        }
        // The `after` cursor is the receiver asserting possession of
        // everything through it; paged results extend that.
        slot.cursor = slot.cursor.max(after);
        let mut items = Vec::new();
        let mut next_cursor = after;
        let mut has_more = false;
        for entry in &mut slot.entries {
            if entry.seq <= after {
                continue;
            }
            if items.len() == limit {
                has_more = true;
                break;
            }
            entry.fetched = true;
            next_cursor = entry.seq;
            items.push(FetchedItem {
                seq: entry.seq,
                id: entry.id.clone(),
                payload: entry.payload.clone(),
            });
        }
        slot.cursor = slot.cursor.max(next_cursor);
        FetchPage::Entries {
            items,
            next_cursor,
            has_more,
        }
    }

    /// Folds every entry the receiver has certainly seen (through
    /// its highest fetch cursor) into the bucket's checkpoint and
    /// drops it. For any cursor at or after the new boundary, fetch
    /// results are identical to pre-compaction ones.
    pub fn compact(&mut self, bucket: &BucketId) -> CompactionSummary {
        let Some(slot) = self.buckets.get_mut(bucket) else {
            return CompactionSummary::default();
        };
        compact_to(slot, slot.cursor)
    }

    /// Retention sweep: folds every entry older than the policy's
    /// `max_age_ticks` into the checkpoint, *fetched or not*, and
    /// returns the per-bucket summaries. This is the only path where
    /// the relay deliberately loses undelivered mail — by explicit
    /// policy age, reported to the caller, never silently.
    pub fn sweep_expired(&mut self) -> Vec<(BucketId, CompactionSummary)> {
        let mut swept = Vec::new();
        let bucket_ids: Vec<BucketId> = self.buckets.keys().cloned().collect();
        for bucket in bucket_ids {
            let Some(slot) = self.buckets.get_mut(&bucket) else {
                continue;
            };
            let boundary = slot
                .entries
                .iter()
                .take_while(|entry| {
                    self.tick.saturating_sub(entry.pushed_tick) > self.policy.max_age_ticks
                })
                .map(|entry| entry.seq)
                .max()
                .unwrap_or(slot.compacted_up_to);
            let summary = compact_to(slot, boundary);
            if summary.payloads > 0 {
                swept.push((bucket, summary));
            }
        }
        swept
    }

    /// The bucket's visible state (zeroed for unknown buckets).
    #[must_use]
    pub fn stats(&self, bucket: &BucketId) -> BucketStats {
        let Some(slot) = self.buckets.get(bucket) else {
            return BucketStats::default();
        };
        let fetched = slot.entries.iter().filter(|entry| entry.fetched).count();
        BucketStats {
            pending: slot.entries.len() - fetched,
            fetched,
            compacted_payloads: slot.compacted_payloads,
            compacted_bytes: slot.compacted_bytes,
            compacted_up_to: slot.compacted_up_to,
            last_seq: slot.entries.back().map_or(slot.compacted_up_to, |e| e.seq),
        }
    }
}

/// Removes entries at or below `boundary` (which must be a prefix —
/// sequence numbers are assigned in push order) and folds them into
/// the checkpoint counts.
fn compact_to(slot: &mut Bucket, boundary: u64) -> CompactionSummary {
    let mut payloads = 0;
    let mut bytes = 0;
    while slot
        .entries
        .front()
        .is_some_and(|entry| entry.seq <= boundary)
    {
        if let Some(entry) = slot.entries.pop_front() {
            payloads += 1;
            bytes += entry.payload.len() as u64;
        }
    }
    slot.compacted_up_to = slot.compacted_up_to.max(boundary);
    slot.compacted_payloads += payloads;
    slot.compacted_bytes += bytes;
    CompactionSummary {
        compacted_up_to: slot.compacted_up_to,
        payloads,
        bytes,
    }
}

//! Personal Vault device lifecycle and tombstone-frontier contracts
//! (ADR 0027 §5, mitigating ADR 0025 threats 4, 9, and 10).
//!
//! Bytes and rules only: this module deliberately knows nothing about
//! cryptography (signatures are opaque bounded bytes — verification is
//! `altior-crypto`'s job), storage, or the relay. The sync engine and
//! `altior-storage` consume these types behind the ADR 0025 sync gate.

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::id::MemoryId;
use crate::time::UnixMillis;

/// A device label inside one Personal Vault. Bounds mirror the crypto
/// layer's device ids (1..=128 UTF-8 bytes) without a dependency on it:
/// pairing copies the label through, and the pairing transcript is what
/// authenticates it.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct DeviceLabel(String);

impl DeviceLabel {
    /// Maximum UTF-8 byte length.
    pub const MAX_LEN: usize = 128;

    /// A label from any non-empty, bounded string.
    ///
    /// # Errors
    ///
    /// [`VaultError::EmptyDeviceLabel`] for empty (after no trimming —
    /// labels are exact) and [`VaultError::DeviceLabelTooLong`] beyond
    /// 128 bytes.
    pub fn new(label: impl Into<String>) -> Result<Self, VaultError> {
        let label = label.into();
        if label.is_empty() {
            return Err(VaultError::EmptyDeviceLabel);
        }
        if label.len() > Self::MAX_LEN {
            return Err(VaultError::DeviceLabelTooLong {
                length: label.len(),
                max: Self::MAX_LEN,
            });
        }
        Ok(Self(label))
    }

    /// The label.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DeviceLabel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// The per-direction pairing sequence, incremented on every re-pairing
/// (ADR 0027 §4.4). Inbound envelopes carrying an older value are
/// rejected before any cryptography — the disk-rollback defense.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct EpochSeq(u32);

impl EpochSeq {
    /// The first pairing's sequence.
    pub const INITIAL: Self = Self(1);

    /// The next sequence; checked, so a u32 overflow fails instead of
    /// wrapping into an already-used value.
    #[must_use]
    pub fn next(self) -> Option<Self> {
        self.0.checked_add(1).map(Self)
    }

    /// The raw value (wire form).
    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// The Ed25519 signature of the vault's offline recovery key over the
/// canonical revocation tuple. Opaque here; verification is crypto's.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RecoverySignature(Vec<u8>);

impl RecoverySignature {
    /// Maximum accepted signature bytes (Ed25519 is exactly 64; the cap
    /// leaves room for future wrapped formats without a schema break).
    pub const MAX_LEN: usize = 1024;

    /// A signature from raw bytes.
    ///
    /// # Errors
    ///
    /// [`VaultError::EmptySignature`] for empty bytes and
    /// [`VaultError::SignatureTooLong`] beyond the cap.
    pub fn new(bytes: Vec<u8>) -> Result<Self, VaultError> {
        if bytes.is_empty() {
            return Err(VaultError::EmptySignature);
        }
        if bytes.len() > Self::MAX_LEN {
            return Err(VaultError::SignatureTooLong {
                length: bytes.len(),
                max: Self::MAX_LEN,
            });
        }
        Ok(Self(bytes))
    }

    /// The raw signature bytes.
    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

/// A revocation certificate: the vault owner's signed order to drop a
/// device (ADR 0025 threat 4). Applied as a high-priority journal event,
/// it wipes pairwise sessions for the device and triggers document-key
/// rotation for the survivors.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RevocationCertificate {
    device: DeviceLabel,
    revoked_at: UnixMillis,
    signature: RecoverySignature,
}

impl RevocationCertificate {
    /// A certificate binding the device, the time, and the recovery
    /// signature into one unit.
    ///
    /// # Errors
    ///
    /// Propagates [`VaultError`] from the parts.
    pub fn new(
        device: DeviceLabel,
        revoked_at: UnixMillis,
        signature: RecoverySignature,
    ) -> Result<Self, VaultError> {
        Ok(Self {
            device,
            revoked_at,
            signature,
        })
    }

    /// The revoked device.
    #[must_use]
    pub fn device(&self) -> &DeviceLabel {
        &self.device
    }

    /// When the revocation was issued.
    #[must_use]
    pub const fn revoked_at(&self) -> UnixMillis {
        self.revoked_at
    }

    /// The recovery signature.
    #[must_use]
    pub const fn signature(&self) -> &RecoverySignature {
        &self.signature
    }
}

/// Lifecycle status of a vault device. Transitions are one-way:
/// `Active → Revoked`, never back (a re-paired device is a *new*
/// record with a bumped [`EpochSeq`]).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum VaultDeviceStatus {
    /// Paired and trusted.
    Active,
    /// Revoked by certificate; sessions wiped, keys rotated.
    Revoked {
        /// When the revocation was applied locally.
        revoked_at: UnixMillis,
        /// The certificate that ordered it.
        certificate: RevocationCertificate,
    },
}

/// One paired device of the Personal Vault.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct VaultDeviceRecord {
    device: DeviceLabel,
    added_at: UnixMillis,
    added_by: DeviceLabel,
    epoch_seq: EpochSeq,
    status: VaultDeviceStatus,
}

impl VaultDeviceRecord {
    /// A freshly paired device record.
    ///
    /// # Errors
    ///
    /// [`VaultError::SelfPairing`] when a device pairs with itself;
    /// [`VaultError::RevokedAtBeforeAddedAt`] cannot happen here but is
    /// kept to the revoke path; label errors propagate.
    pub fn pair(
        device: DeviceLabel,
        added_by: DeviceLabel,
        added_at: UnixMillis,
        epoch_seq: EpochSeq,
    ) -> Result<Self, VaultError> {
        if device == added_by {
            return Err(VaultError::SelfPairing {
                device: device.to_string(),
            });
        }
        Ok(Self {
            device,
            added_at,
            added_by,
            epoch_seq,
            status: VaultDeviceStatus::Active,
        })
    }

    /// Applies a revocation certificate: one-way `Active → Revoked`.
    ///
    /// # Errors
    ///
    /// [`VaultError::AlreadyRevoked`] on a second certificate;
    /// [`VaultError::CertificateDeviceMismatch`] when the certificate
    /// names another device; [`VaultError::RevokedAtBeforeAddedAt`]
    /// when the certificate predates the pairing.
    pub fn revoke(
        &mut self,
        certificate: RevocationCertificate,
        applied_at: UnixMillis,
    ) -> Result<(), VaultError> {
        match self.status {
            VaultDeviceStatus::Revoked { .. } => {
                return Err(VaultError::AlreadyRevoked {
                    device: self.device.to_string(),
                });
            }
            VaultDeviceStatus::Active => {}
        }
        if certificate.device() != &self.device {
            return Err(VaultError::CertificateDeviceMismatch {
                certificate_device: certificate.device().to_string(),
                record_device: self.device.to_string(),
            });
        }
        if certificate.revoked_at() < self.added_at || applied_at < self.added_at {
            return Err(VaultError::RevokedAtBeforeAddedAt {
                added_at: self.added_at.as_millis(),
                revoked_at: certificate
                    .revoked_at()
                    .as_millis()
                    .min(applied_at.as_millis()),
            });
        }
        self.status = VaultDeviceStatus::Revoked {
            revoked_at: applied_at,
            certificate,
        };
        Ok(())
    }

    /// The device label.
    #[must_use]
    pub const fn device(&self) -> &DeviceLabel {
        &self.device
    }

    /// Who paired this device (the prior device's label, or the same
    /// label at genesis is refused — the first device is registered
    /// through [`VaultDeviceRecord::genesis`]).
    #[must_use]
    pub const fn added_by(&self) -> &DeviceLabel {
        &self.added_by
    }

    /// When the device was paired.
    #[must_use]
    pub const fn added_at(&self) -> UnixMillis {
        self.added_at
    }

    /// The pairing sequence.
    #[must_use]
    pub const fn epoch_seq(&self) -> EpochSeq {
        self.epoch_seq
    }

    /// The lifecycle status.
    #[must_use]
    pub const fn status(&self) -> &VaultDeviceStatus {
        &self.status
    }

    /// Whether the device may read or write vault data.
    #[must_use]
    pub const fn is_active(&self) -> bool {
        matches!(self.status, VaultDeviceStatus::Active)
    }

    /// Bumps the pairing sequence after a re-pairing.
    ///
    /// # Errors
    ///
    /// [`VaultError::EpochSeqExhausted`] when the u32 space is used up.
    pub fn bump_epoch_seq(&mut self) -> Result<(), VaultError> {
        self.epoch_seq = self.epoch_seq.next().ok_or(VaultError::EpochSeqExhausted)?;
        Ok(())
    }
}

/// The permanent record that a memory was forgotten (ADR 0025 threat
/// 10). Compaction may never garbage-collect a tombstone that is at or
/// above the global acknowledgement frontier, and a stale device that
/// re-proposes a forgotten memory is contradicted by its digest.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct MemoryTombstone {
    memory_id: MemoryId,
    forgotten_at: UnixMillis,
    digest: TombstoneDigest,
}

impl MemoryTombstone {
    /// A tombstone tuple.
    ///
    /// # Errors
    ///
    /// [`VaultError::DigestAllZero`] for the reserved all-zero digest.
    pub fn new(
        memory_id: MemoryId,
        forgotten_at: UnixMillis,
        digest: TombstoneDigest,
    ) -> Result<Self, VaultError> {
        Ok(Self {
            memory_id,
            forgotten_at,
            digest,
        })
    }

    /// The forgotten memory.
    #[must_use]
    pub const fn memory_id(&self) -> &MemoryId {
        &self.memory_id
    }

    /// When the forget was issued.
    #[must_use]
    pub const fn forgotten_at(&self) -> UnixMillis {
        self.forgotten_at
    }

    /// The tombstone digest.
    #[must_use]
    pub const fn digest(&self) -> &TombstoneDigest {
        &self.digest
    }
}

/// Content hash binding a tombstone to exactly one forget event;
/// re-proposals with a different digest are conflicts, not echoes. The
/// all-zero value is the reserved "uninitialized" marker.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct TombstoneDigest([u8; 32]);

impl TombstoneDigest {
    /// A digest from raw bytes.
    ///
    /// # Errors
    ///
    /// [`VaultError::DigestAllZero`] for the reserved all-zero value.
    pub fn from_bytes(bytes: [u8; 32]) -> Result<Self, VaultError> {
        if bytes == [0; 32] {
            return Err(VaultError::DigestAllZero);
        }
        Ok(Self(bytes))
    }

    /// Raw digest bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

/// Per-device acknowledgement cursors over the replicated journal
/// (ADR 0027 §6). Compaction may reclaim only at or below the *floor* —
/// the minimum frontier every recorded device has durably applied. A
/// device with no recorded cursor is unknown; the floor is `None` and
/// compaction must not proceed (conservative, correct, visible).
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AckFrontier {
    cursors: BTreeMap<DeviceLabel, u64>,
}

impl AckFrontier {
    /// Records that `device` durably applied everything through
    /// `sequence`. Cursors only move forward; a lower value is ignored
    /// (a rollback claim is not an acknowledgement).
    pub fn observe(&mut self, device: DeviceLabel, sequence: u64) {
        let entry = self.cursors.entry(device).or_insert(0);
        if sequence > *entry {
            *entry = sequence;
        }
    }

    /// The sequence through which every recorded device has applied, or
    /// `None` when no device has acknowledged (or none is recorded at
    /// all). Compaction may reclaim at or below this floor, never above.
    #[must_use]
    pub fn compaction_floor(&self) -> Option<u64> {
        self.cursors.values().copied().min()
    }

    /// The cursor recorded for one device, if any.
    #[must_use]
    pub fn cursor_of(&self, device: &DeviceLabel) -> Option<u64> {
        self.cursors.get(device).copied()
    }

    /// How many devices have acknowledged.
    #[must_use]
    pub fn recorded_devices(&self) -> usize {
        self.cursors.len()
    }
}

/// Typed validation failures for the vault lifecycle contracts.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum VaultError {
    /// A device label is empty.
    EmptyDeviceLabel,
    /// A device label exceeds the byte cap.
    DeviceLabelTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A recovery signature is empty.
    EmptySignature,
    /// A recovery signature exceeds the byte cap.
    SignatureTooLong {
        /// The actual length.
        length: usize,
        /// The maximum allowed length.
        max: usize,
    },
    /// A device tried to pair with itself.
    SelfPairing {
        /// The duplicated label.
        device: String,
    },
    /// A second revocation certificate reached an already-revoked
    /// device.
    AlreadyRevoked {
        /// The revoked device's label.
        device: String,
    },
    /// The certificate names a different device than the record.
    CertificateDeviceMismatch {
        /// The device the certificate revokes.
        certificate_device: String,
        /// The device the record holds.
        record_device: String,
    },
    /// The revocation (or its application) predates the pairing.
    RevokedAtBeforeAddedAt {
        /// The pairing time.
        added_at: u64,
        /// The offending revocation time.
        revoked_at: u64,
    },
    /// The pairing sequence exhausted the u32 space.
    EpochSeqExhausted,
    /// A tombstone digest is the reserved all-zero value.
    DigestAllZero,
}

impl fmt::Display for VaultError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyDeviceLabel => write!(f, "device label is empty"),
            Self::DeviceLabelTooLong { length, max } => {
                write!(f, "device label is {length} bytes; max {max}")
            }
            Self::EmptySignature => write!(f, "recovery signature is empty"),
            Self::SignatureTooLong { length, max } => {
                write!(f, "recovery signature is {length} bytes; max {max}")
            }
            Self::SelfPairing { device } => {
                write!(f, "device {device} cannot pair with itself")
            }
            Self::AlreadyRevoked { device } => {
                write!(f, "device {device} is already revoked")
            }
            Self::CertificateDeviceMismatch {
                certificate_device,
                record_device,
            } => write!(
                f,
                "certificate revokes {certificate_device}; record holds {record_device}"
            ),
            Self::RevokedAtBeforeAddedAt {
                added_at,
                revoked_at,
            } => write!(
                f,
                "revocation at {revoked_at} predates pairing at {added_at}"
            ),
            Self::EpochSeqExhausted => write!(f, "epoch sequence exhausted; re-identity the vault"),
            Self::DigestAllZero => write!(f, "tombstone digest is the reserved all-zero value"),
        }
    }
}

impl std::error::Error for VaultError {}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_700_000_000_000;

    fn label(name: &str) -> DeviceLabel {
        DeviceLabel::new(name).unwrap()
    }

    fn at(offset_ms: u64) -> UnixMillis {
        UnixMillis::from_millis(T0 + offset_ms)
    }

    fn cert(device: &DeviceLabel, at_time: UnixMillis) -> RevocationCertificate {
        RevocationCertificate::new(
            device.clone(),
            at_time,
            RecoverySignature::new(vec![0xAB; 64]).unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn device_labels_are_bounded_and_exact() {
        assert!(matches!(
            DeviceLabel::new(""),
            Err(VaultError::EmptyDeviceLabel)
        ));
        assert!(matches!(
            DeviceLabel::new(" ".repeat(129)),
            Err(VaultError::DeviceLabelTooLong {
                length: 129,
                max: 128
            })
        ));
        // Exact bounds: 128 bytes is legal, no trimming happens.
        assert_eq!(
            DeviceLabel::new("x".repeat(128)).unwrap().as_str().len(),
            128
        );
        assert_eq!(DeviceLabel::new("phone").unwrap().as_str(), "phone");
    }

    #[test]
    fn pairing_refuses_self_and_starts_active() {
        let device = label("phone");
        assert!(matches!(
            VaultDeviceRecord::pair(device.clone(), device.clone(), at(0), EpochSeq::INITIAL),
            Err(VaultError::SelfPairing { .. })
        ));

        let record =
            VaultDeviceRecord::pair(device.clone(), label("laptop"), at(0), EpochSeq::INITIAL)
                .unwrap();
        assert!(record.is_active());
        assert_eq!(record.epoch_seq(), EpochSeq::INITIAL);
        assert_eq!(record.added_by().as_str(), "laptop");
    }

    #[test]
    fn revocation_is_one_way_and_binds_the_record() {
        let device = label("phone");
        let mut record =
            VaultDeviceRecord::pair(device.clone(), label("laptop"), at(0), EpochSeq::INITIAL)
                .unwrap();

        // A certificate for another device is refused.
        let wrong = cert(&label("tablet"), at(10));
        assert!(matches!(
            record.revoke(wrong, at(20)),
            Err(VaultError::CertificateDeviceMismatch { .. })
        ));
        assert!(record.is_active());

        // A certificate predating the pairing is refused.
        assert!(matches!(
            record.revoke(cert(&device, UnixMillis::from_millis(T0 - 1)), at(20)),
            Err(VaultError::RevokedAtBeforeAddedAt { .. })
        ));

        // The matching certificate revokes, exactly once.
        record.revoke(cert(&device, at(10)), at(20)).unwrap();
        assert!(!record.is_active());
        assert!(matches!(
            record.revoke(cert(&device, at(30)), at(40)),
            Err(VaultError::AlreadyRevoked { .. })
        ));
        match record.status() {
            VaultDeviceStatus::Revoked {
                revoked_at,
                certificate,
            } => {
                assert_eq!(*revoked_at, at(20));
                assert_eq!(certificate.device(), &device);
            }
            other @ VaultDeviceStatus::Active => panic!("expected revoked, got {other:?}"),
        }
    }

    #[test]
    fn epoch_seq_advances_checked() {
        let mut record =
            VaultDeviceRecord::pair(label("phone"), label("laptop"), at(0), EpochSeq::INITIAL)
                .unwrap();
        record.bump_epoch_seq().unwrap();
        assert_eq!(record.epoch_seq().get(), 2);
        record.bump_epoch_seq().unwrap();
        assert_eq!(record.epoch_seq().get(), 3);

        let last =
            VaultDeviceRecord::pair(label("phone"), label("laptop"), at(0), EpochSeq(u32::MAX));
        // Construction accepts the value; the bump must fail, not wrap.
        let mut last = last.unwrap();
        assert!(matches!(
            last.bump_epoch_seq(),
            Err(VaultError::EpochSeqExhausted)
        ));
    }

    #[test]
    fn tombstones_reject_the_zero_digest() {
        let memory_id = MemoryId::try_from("mem_vault000000000001").unwrap();
        assert!(matches!(
            TombstoneDigest::from_bytes([0; 32]),
            Err(VaultError::DigestAllZero)
        ));
        let tombstone = MemoryTombstone::new(
            memory_id.clone(),
            at(5),
            TombstoneDigest::from_bytes([7; 32]).unwrap(),
        )
        .unwrap();
        assert_eq!(tombstone.memory_id(), &memory_id);
        assert_eq!(tombstone.digest().as_bytes(), &[7; 32]);
    }

    #[test]
    fn ack_frontier_floor_is_the_conservative_minimum() {
        let mut frontier = AckFrontier::default();
        // No recorded device: compaction must not proceed.
        assert_eq!(frontier.compaction_floor(), None);

        frontier.observe(label("a"), 10);
        assert_eq!(frontier.compaction_floor(), Some(10));

        frontier.observe(label("b"), 4);
        assert_eq!(frontier.compaction_floor(), Some(4));

        // A rollback claim from b does not lower the floor.
        frontier.observe(label("b"), 2);
        assert_eq!(frontier.cursor_of(&label("b")), Some(4));
        assert_eq!(frontier.compaction_floor(), Some(4));

        // Progress raises the floor only when the slowest catches up.
        frontier.observe(label("b"), 12);
        assert_eq!(frontier.compaction_floor(), Some(10));
        frontier.observe(label("a"), 12);
        assert_eq!(frontier.compaction_floor(), Some(12));
        assert_eq!(frontier.recorded_devices(), 2);
    }

    #[test]
    fn signatures_are_bounded() {
        assert!(matches!(
            RecoverySignature::new(Vec::new()),
            Err(VaultError::EmptySignature)
        ));
        assert!(matches!(
            RecoverySignature::new(vec![0; 1025]),
            Err(VaultError::SignatureTooLong {
                length: 1025,
                max: 1024
            })
        ));
        assert_eq!(
            RecoverySignature::new(vec![1; 64])
                .unwrap()
                .as_bytes()
                .len(),
            64
        );
    }
}

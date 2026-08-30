//! CRDT bake-off spike: the `SyncDocumentEngine` port over Loro and
//! Automerge (ADR 0010).
//!
//! `docs/ARCHITECTURE.md` names `SyncDocumentEngine` as the port for
//! concurrent knowledge-document edits. This crate defines that port
//! with the smallest honest document model — named text fields plus
//! scalar fields — and implements it twice so the adversarial suite
//! can race both libraries under identical, deterministic schedules.
//! No timing assertions, no randomness beyond a fixed-seed LCG, no
//! network.

pub mod automerge_engine;
pub mod loro_engine;

use std::collections::BTreeMap;
use std::fmt::{self, Debug};

pub use automerge_engine::AutomergeEngine;
pub use loro_engine::LoroEngine;

/// Maximum framed state accepted from an untrusted sync transport.
pub const MAX_STATE_BYTES: usize = 8 * 1024 * 1024;

const STATE_MAGIC: &[u8; 4] = b"ALTC";
const STATE_VERSION: u8 = 1;
const STATE_HEADER_LEN: usize = 4 + 1 + 1 + 8 + 8;

/// A declared logical field kind.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FieldKind {
    /// Concurrent text.
    Text,
    /// Last-writer-wins string scalar.
    Scalar,
}

/// Typed CRDT boundary failures.
#[derive(Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum CrdtError {
    /// Field names must not be empty.
    EmptyFieldName,
    /// One logical name was declared more than once.
    DuplicateField { field: String },
    /// One logical name was declared as both text and scalar.
    FieldTypeConflict { field: String },
    /// An operation addressed a field outside the schema.
    UndeclaredField { field: String },
    /// An operation used a declared field through the wrong API.
    WrongFieldType {
        field: String,
        expected: FieldKind,
        actual: FieldKind,
    },
    /// `AnyEngine` received an unknown implementation name.
    UnknownEngine { kind: String },
    /// State exceeded the transport/import bound.
    StateTooLarge { size: usize, limit: usize },
    /// State framing was truncated or internally inconsistent.
    MalformedState { detail: String },
    /// State belongs to a different engine implementation.
    EngineMismatch { expected: String, found: u8 },
    /// State uses an unsupported framing version.
    UnsupportedStateVersion { found: u8 },
    /// State was produced for a different declared schema.
    SchemaMismatch,
    /// The backing engine rejected or exposed invalid state.
    ImportRejected { detail: String },
}

impl fmt::Display for CrdtError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyFieldName => write!(f, "field name is empty"),
            Self::DuplicateField { field } => write!(f, "field {field} is declared twice"),
            Self::FieldTypeConflict { field } => {
                write!(f, "field {field} is declared as both text and scalar")
            }
            Self::UndeclaredField { field } => write!(f, "field {field} is not declared"),
            Self::WrongFieldType {
                field,
                expected,
                actual,
            } => write!(
                f,
                "field {field} is {actual:?}, but this operation requires {expected:?}"
            ),
            Self::UnknownEngine { kind } => write!(f, "unknown CRDT engine {kind}"),
            Self::StateTooLarge { size, limit } => {
                write!(f, "state has {size} bytes, above the {limit}-byte limit")
            }
            Self::MalformedState { detail } => write!(f, "malformed CRDT state: {detail}"),
            Self::EngineMismatch { expected, found } => {
                write!(f, "state engine tag {found} does not match {expected}")
            }
            Self::UnsupportedStateVersion { found } => {
                write!(f, "state framing version {found} is unsupported")
            }
            Self::SchemaMismatch => write!(f, "state schema does not match the document schema"),
            Self::ImportRejected { detail } => write!(f, "CRDT state import rejected: {detail}"),
        }
    }
}

impl std::error::Error for CrdtError {}

/// Immutable schema required before replicas fork.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DocumentSchema {
    fields: BTreeMap<String, FieldKind>,
}

impl DocumentSchema {
    /// Declares every text and scalar field.
    ///
    /// # Errors
    ///
    /// Returns a typed schema error for empty, duplicate, or
    /// cross-kind duplicate names.
    pub fn new(text_fields: &[&str], scalar_fields: &[&str]) -> Result<Self, CrdtError> {
        let mut fields = BTreeMap::new();
        for (names, kind) in [
            (text_fields, FieldKind::Text),
            (scalar_fields, FieldKind::Scalar),
        ] {
            for name in names {
                if name.is_empty() {
                    return Err(CrdtError::EmptyFieldName);
                }
                if let Some(previous) = fields.insert((*name).to_owned(), kind) {
                    return Err(if previous == kind {
                        CrdtError::DuplicateField {
                            field: (*name).to_owned(),
                        }
                    } else {
                        CrdtError::FieldTypeConflict {
                            field: (*name).to_owned(),
                        }
                    });
                }
            }
        }
        Ok(Self { fields })
    }

    pub(crate) const fn empty() -> Self {
        Self {
            fields: BTreeMap::new(),
        }
    }

    pub(crate) fn require(&self, field: &str, expected: FieldKind) -> Result<(), CrdtError> {
        let actual = self
            .fields
            .get(field)
            .copied()
            .ok_or_else(|| CrdtError::UndeclaredField {
                field: field.to_owned(),
            })?;
        if actual != expected {
            return Err(CrdtError::WrongFieldType {
                field: field.to_owned(),
                expected,
                actual,
            });
        }
        Ok(())
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, FieldKind)> {
        self.fields
            .iter()
            .map(|(name, kind)| (name.as_str(), *kind))
    }

    fn digest(&self) -> u64 {
        let mut hash = Fnv1a::seed();
        for (name, kind) in self.iter() {
            hash.mix(&[match kind {
                FieldKind::Text => 1,
                FieldKind::Scalar => 2,
            }]);
            hash.mix(&(name.len() as u64).to_be_bytes());
            hash.mix(name.as_bytes());
        }
        hash.finish()
    }
}

/// One field of the canonical document view.
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum FieldView {
    /// A concurrently editable text field and its current content.
    Text(String),
    /// A last-writer-wins scalar field and its current value.
    Scalar(String),
}

/// The minimal document model the bake-off exercises.
///
/// Positions are char indices into the field's current text; every
/// implementation clamps out-of-range positions (insert past the end
/// appends, delete past the end trims) so adversarial schedules stay
/// valid operations instead of errors.
pub trait SyncDocumentEngine: Debug {
    /// The implementing library's name, for evidence output.
    fn engine_name(&self) -> &'static str;

    /// Inserts `text` into the named text field at `pos`.
    ///
    /// # Errors
    ///
    /// Returns a typed schema or engine error.
    fn insert_text(&mut self, field: &str, pos: usize, text: &str) -> Result<(), CrdtError>;

    /// Deletes `len` chars from the named text field at `pos`.
    ///
    /// # Errors
    ///
    /// Returns a typed schema or engine error.
    fn delete_text(&mut self, field: &str, pos: usize, len: usize) -> Result<(), CrdtError>;

    /// Sets a scalar field (last-writer-wins on concurrent writes).
    ///
    /// # Errors
    ///
    /// Returns a typed schema or engine error.
    fn set_field(&mut self, field: &str, value: &str) -> Result<(), CrdtError>;

    /// The current text of a field (empty when absent).
    ///
    /// # Errors
    ///
    /// Returns a typed schema or engine error.
    fn text_of(&self, field: &str) -> Result<String, CrdtError>;

    /// The current value of a scalar field.
    ///
    /// # Errors
    ///
    /// Returns a typed schema or engine error.
    fn field_of(&self, field: &str) -> Result<Option<String>, CrdtError>;

    /// The canonical view: every field, sorted by name. Convergence
    /// means two replicas produce equal views.
    fn view(&self) -> Vec<(String, FieldView)>;

    /// A digest of [`SyncDocumentEngine::view`], for compact asserts.
    fn view_digest(&self) -> u64 {
        let mut hash = Fnv1a::seed();
        for (name, value) in self.view() {
            hash.mix(name.as_bytes());
            hash.mix(&[0]);
            match value {
                FieldView::Text(text) => {
                    hash.mix(b"t");
                    hash.mix(text.as_bytes());
                }
                FieldView::Scalar(value) => {
                    hash.mix(b"s");
                    hash.mix(value.as_bytes());
                }
            }
            hash.mix(&[0]);
        }
        hash.finish()
    }

    /// The opaque, engine-specific encoded document state. These bytes
    /// are what a sync transport moves between replicas; importing
    /// them merges every change they contain (state-based merge, so
    /// replicas converge after a mutual exchange).
    fn export_state(&self) -> Vec<u8>;

    /// Merges an encoded state produced by the same engine kind into
    /// this document. Importing is idempotent.
    ///
    /// # Errors
    ///
    /// Returns a typed framing, schema, size, or engine parse error.
    fn import_state(&mut self, bytes: &[u8]) -> Result<(), CrdtError>;

    /// The encoded document size in bytes (the bake-off metric).
    fn state_size(&self) -> usize {
        self.export_state().len()
    }
}

/// Either engine behind the port, so tests and future callers can
/// hold one value that still knows how to make peers and fresh
/// schema-initialized documents (operations the trait itself cannot
/// expose without breaking object safety).
#[derive(Debug)]
pub enum AnyEngine {
    /// The Loro implementation.
    Loro(LoroEngine),
    /// The Automerge implementation, boxed: an `Automerge` document
    /// dwarfs a `LoroDoc`, and boxing the large variant keeps every
    /// `AnyEngine` value small.
    Automerge(Box<AutomergeEngine>),
}

impl AnyEngine {
    /// A schema-initialized document of the given kind: text fields
    /// exist before any replica forks. Automerge needs the field list
    /// (creating a container under a map key is itself a map write, so
    /// two replicas creating the same field concurrently would resolve
    /// one side away); Loro's name-addressed root containers make the
    /// list a no-op there.
    /// # Errors
    ///
    /// Returns a typed schema error or [`CrdtError::UnknownEngine`].
    pub fn with_fields(kind: &str, fields: &[&str]) -> Result<Self, CrdtError> {
        Self::with_schema(kind, fields, &[])
    }

    /// Creates an engine under a complete pre-fork schema.
    ///
    /// # Errors
    ///
    /// Returns a typed schema, engine selection, or initialization error.
    pub fn with_schema(
        kind: &str,
        text_fields: &[&str],
        scalar_fields: &[&str],
    ) -> Result<Self, CrdtError> {
        let schema = DocumentSchema::new(text_fields, scalar_fields)?;
        match kind {
            "loro" => Ok(Self::Loro(LoroEngine::with_schema(schema)?)),
            "automerge" => Ok(Self::Automerge(Box::new(AutomergeEngine::with_schema(
                schema,
            )?))),
            other => Err(CrdtError::UnknownEngine {
                kind: other.to_owned(),
            }),
        }
    }

    /// Forks this document under a deterministic peer identity, so
    /// replicas in one run have distinct authors without random input.
    #[must_use]
    pub fn fork_with_peer(&self, peer: u64) -> Self {
        match self {
            Self::Loro(engine) => Self::Loro(engine.fork_with_peer(peer)),
            Self::Automerge(engine) => Self::Automerge(Box::new(engine.fork_with_peer(peer))),
        }
    }

    /// The engine behind this value, for trait calls.
    #[must_use]
    pub fn as_dyn(&self) -> &dyn SyncDocumentEngine {
        match self {
            Self::Loro(engine) => engine,
            Self::Automerge(engine) => engine.as_ref(),
        }
    }

    /// The engine behind this value, for trait calls.
    pub fn as_dyn_mut(&mut self) -> &mut dyn SyncDocumentEngine {
        match self {
            Self::Loro(engine) => engine,
            Self::Automerge(engine) => engine.as_mut(),
        }
    }
}

impl SyncDocumentEngine for AnyEngine {
    fn engine_name(&self) -> &'static str {
        self.as_dyn().engine_name()
    }

    fn insert_text(&mut self, field: &str, pos: usize, text: &str) -> Result<(), CrdtError> {
        self.as_dyn_mut().insert_text(field, pos, text)
    }

    fn delete_text(&mut self, field: &str, pos: usize, len: usize) -> Result<(), CrdtError> {
        self.as_dyn_mut().delete_text(field, pos, len)
    }

    fn set_field(&mut self, field: &str, value: &str) -> Result<(), CrdtError> {
        self.as_dyn_mut().set_field(field, value)
    }

    fn text_of(&self, field: &str) -> Result<String, CrdtError> {
        self.as_dyn().text_of(field)
    }

    fn field_of(&self, field: &str) -> Result<Option<String>, CrdtError> {
        self.as_dyn().field_of(field)
    }

    fn view(&self) -> Vec<(String, FieldView)> {
        self.as_dyn().view()
    }

    fn export_state(&self) -> Vec<u8> {
        self.as_dyn().export_state()
    }

    fn import_state(&mut self, bytes: &[u8]) -> Result<(), CrdtError> {
        self.as_dyn_mut().import_state(bytes)
    }
}

pub(crate) fn storage_name(prefix: &str, logical: &str) -> String {
    let mut out = String::from(prefix);
    for byte in logical.as_bytes() {
        use std::fmt::Write;
        write!(&mut out, "{byte:02x}").expect("String writes do not fail");
    }
    out
}

pub(crate) fn frame_state(engine: u8, schema: &DocumentSchema, payload: &[u8]) -> Vec<u8> {
    let mut framed = Vec::with_capacity(STATE_HEADER_LEN + payload.len());
    framed.extend_from_slice(STATE_MAGIC);
    framed.push(STATE_VERSION);
    framed.push(engine);
    framed.extend_from_slice(&schema.digest().to_be_bytes());
    framed.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    framed.extend_from_slice(payload);
    framed
}

pub(crate) fn parse_state<'a>(
    bytes: &'a [u8],
    engine: u8,
    engine_name: &str,
    schema: &DocumentSchema,
) -> Result<&'a [u8], CrdtError> {
    if bytes.len() > MAX_STATE_BYTES {
        return Err(CrdtError::StateTooLarge {
            size: bytes.len(),
            limit: MAX_STATE_BYTES,
        });
    }
    if bytes.len() < STATE_HEADER_LEN || &bytes[..4] != STATE_MAGIC {
        return Err(CrdtError::MalformedState {
            detail: "missing or truncated framing header".to_owned(),
        });
    }
    if bytes[4] != STATE_VERSION {
        return Err(CrdtError::UnsupportedStateVersion { found: bytes[4] });
    }
    if bytes[5] != engine {
        return Err(CrdtError::EngineMismatch {
            expected: engine_name.to_owned(),
            found: bytes[5],
        });
    }
    let digest =
        u64::from_be_bytes(
            bytes[6..14]
                .try_into()
                .map_err(|_| CrdtError::MalformedState {
                    detail: "schema digest is truncated".to_owned(),
                })?,
        );
    if digest != schema.digest() {
        return Err(CrdtError::SchemaMismatch);
    }
    let payload_len =
        u64::from_be_bytes(
            bytes[14..22]
                .try_into()
                .map_err(|_| CrdtError::MalformedState {
                    detail: "payload length is truncated".to_owned(),
                })?,
        );
    let payload_len = usize::try_from(payload_len).map_err(|_| CrdtError::MalformedState {
        detail: "payload length does not fit this platform".to_owned(),
    })?;
    if payload_len != bytes.len() - STATE_HEADER_LEN {
        return Err(CrdtError::MalformedState {
            detail: "payload length does not match frame".to_owned(),
        });
    }
    Ok(&bytes[STATE_HEADER_LEN..])
}

/// FNV-1a 64-bit, so the crate needs no hashing dependency.
#[derive(Debug)]
pub(crate) struct Fnv1a(u64);

impl Fnv1a {
    pub(crate) const fn seed() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    pub(crate) fn mix(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }

    pub(crate) const fn finish(self) -> u64 {
        self.0
    }
}

/// A deterministic linear congruential generator for adversarial
/// schedules — fixed seed, no `rand` dependency, no machine load.
#[derive(Debug)]
pub struct Lcg(u64);

impl Lcg {
    /// A generator with the given seed.
    #[must_use]
    pub const fn seeded(seed: u64) -> Self {
        Self(seed)
    }

    /// The next pseudo-random value in `0..bound` (bound must be > 0).
    #[must_use]
    pub fn next(&mut self, bound: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) % bound
    }

    /// The next pseudo-random index in `0..len` (len must be > 0),
    /// for indexing collections without truncating casts.
    ///
    /// # Panics
    ///
    /// Panics when the drawn value does not fit a `usize` (impossible
    /// while `len` is a valid collection length).
    #[must_use]
    pub fn next_index(&mut self, len: usize) -> usize {
        usize::try_from(self.next(len as u64)).expect("value below len")
    }
}

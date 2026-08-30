//! The Automerge implementation of [`SyncDocumentEngine`] (ADR 0010).

use automerge::transaction::Transactable;
use automerge::{ActorId, Automerge, ObjId, ObjType, ReadDoc, ScalarValue, Value};

use crate::{
    CrdtError, DocumentSchema, FieldKind, FieldView, SyncDocumentEngine, frame_state, parse_state,
    storage_name,
};

const TEXT_PREFIX: &str = "__altior_crdt_v1_text_";
const SCALAR_PREFIX: &str = "__altior_crdt_v1_scalar_";
const ENGINE_TAG: u8 = 2;

/// `SyncDocumentEngine` over an [`Automerge`] document.
///
/// Root keys hold text objects directly; scalar fields are strings
/// stored under `s:<name>` so the canonical view can tell the kinds
/// apart without consulting object metadata. Positions are unicode
/// scalar indices, matching Automerge 0.11's default text encoding.
#[derive(Debug)]
pub struct AutomergeEngine {
    doc: Automerge,
    schema: DocumentSchema,
}

impl AutomergeEngine {
    /// A fresh empty document.
    #[must_use]
    pub fn new() -> Self {
        Self {
            doc: Automerge::new(),
            schema: DocumentSchema::empty(),
        }
    }

    /// A document whose text fields already exist, created before any
    /// replica forks so container creation can never race (see the
    /// trait docs).
    ///
    /// # Errors
    ///
    /// Returns a typed schema or engine initialization error.
    pub fn with_fields(fields: &[&str]) -> Result<Self, CrdtError> {
        Self::with_schema(DocumentSchema::new(fields, &[])?)
    }

    /// A document under a complete immutable schema.
    ///
    /// # Errors
    ///
    /// Returns a typed engine initialization error.
    pub fn with_schema(schema: DocumentSchema) -> Result<Self, CrdtError> {
        let mut engine = Self::new();
        engine.schema = schema;
        if engine
            .schema
            .iter()
            .any(|(_, kind)| kind == FieldKind::Text)
        {
            let mut tx = engine.doc.transaction();
            for (field, kind) in engine.schema.iter() {
                if kind == FieldKind::Text {
                    tx.put_object(
                        automerge::ROOT,
                        storage_name(TEXT_PREFIX, field),
                        ObjType::Text,
                    )
                    .map_err(|error| CrdtError::ImportRejected {
                        detail: format!("automerge schema creation failed: {error}"),
                    })?;
                }
            }
            tx.commit();
        }
        Ok(engine)
    }

    /// Forks this document under a deterministic peer identity.
    /// `Automerge::fork`'s random actor would make tie-breaks
    /// nondeterministic across test runs.
    #[must_use]
    pub fn fork_with_peer(&self, peer: u64) -> Self {
        let mut doc = self.doc.clone();
        doc.set_actor(ActorId::from(peer.to_le_bytes().to_vec()));
        Self {
            doc,
            schema: self.schema.clone(),
        }
    }

    fn text_object(&self, field: &str) -> Result<ObjId, CrdtError> {
        self.schema.require(field, FieldKind::Text)?;
        let stored = storage_name(TEXT_PREFIX, field);
        if let Some((Value::Object(ObjType::Text), id)) = self
            .doc
            .get(automerge::ROOT, &stored)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge root read failed: {error}"),
            })?
        {
            return Ok(id);
        }
        Err(CrdtError::ImportRejected {
            detail: format!("declared text field {field} is missing from state"),
        })
    }
}

impl Default for AutomergeEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncDocumentEngine for AutomergeEngine {
    fn engine_name(&self) -> &'static str {
        "automerge"
    }

    fn insert_text(&mut self, field: &str, pos: usize, text: &str) -> Result<(), CrdtError> {
        let object = self.text_object(field)?;
        if text.is_empty() {
            return Ok(());
        }
        let len = self
            .doc
            .text(&object)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge text read failed: {error}"),
            })?
            .chars()
            .count();
        let pos = pos.min(len);
        let mut tx = self.doc.transaction();
        tx.splice_text(object, pos, 0, text)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge text insert failed: {error}"),
            })?;
        tx.commit();
        Ok(())
    }

    fn delete_text(&mut self, field: &str, pos: usize, len: usize) -> Result<(), CrdtError> {
        let object = self.text_object(field)?;
        let total = self
            .doc
            .text(&object)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge text read failed: {error}"),
            })?
            .chars()
            .count();
        let start = pos.min(total);
        let end = (pos.saturating_add(len)).min(total);
        if end == start {
            return Ok(());
        }
        let mut tx = self.doc.transaction();
        tx.splice_text(
            object,
            start,
            isize::try_from(end - start).expect("delete count fits"),
            "",
        )
        .map_err(|error| CrdtError::ImportRejected {
            detail: format!("automerge text delete failed: {error}"),
        })?;
        tx.commit();
        Ok(())
    }

    fn set_field(&mut self, field: &str, value: &str) -> Result<(), CrdtError> {
        self.schema.require(field, FieldKind::Scalar)?;
        let mut tx = self.doc.transaction();
        tx.put(automerge::ROOT, storage_name(SCALAR_PREFIX, field), value)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge scalar put failed: {error}"),
            })?;
        tx.commit();
        Ok(())
    }

    fn text_of(&self, field: &str) -> Result<String, CrdtError> {
        self.schema.require(field, FieldKind::Text)?;
        let stored = storage_name(TEXT_PREFIX, field);
        let Some((Value::Object(ObjType::Text), id)) = self
            .doc
            .get(automerge::ROOT, &stored)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge root read failed: {error}"),
            })?
        else {
            return Err(CrdtError::ImportRejected {
                detail: format!("declared text field {field} is missing from state"),
            });
        };
        self.doc
            .text(&id)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge text read failed: {error}"),
            })
    }

    fn field_of(&self, field: &str) -> Result<Option<String>, CrdtError> {
        self.schema.require(field, FieldKind::Scalar)?;
        Ok(
            match self
                .doc
                .get(automerge::ROOT, storage_name(SCALAR_PREFIX, field))
                .map_err(|error| CrdtError::ImportRejected {
                    detail: format!("automerge root read failed: {error}"),
                })? {
                Some((Value::Scalar(value), _)) => match value.as_ref() {
                    ScalarValue::Str(value) => Some(value.to_string()),
                    _ => None,
                },
                _ => None,
            },
        )
    }

    fn view(&self) -> Vec<(String, FieldView)> {
        self.schema
            .iter()
            .map(|(name, kind)| match kind {
                FieldKind::Text => (
                    name.to_owned(),
                    FieldView::Text(self.text_of(name).unwrap_or_default()),
                ),
                FieldKind::Scalar => (
                    name.to_owned(),
                    FieldView::Scalar(self.field_of(name).ok().flatten().unwrap_or_default()),
                ),
            })
            .collect()
    }

    fn export_state(&self) -> Vec<u8> {
        frame_state(ENGINE_TAG, &self.schema, &self.doc.save())
    }

    fn import_state(&mut self, bytes: &[u8]) -> Result<(), CrdtError> {
        let payload = parse_state(bytes, ENGINE_TAG, self.engine_name(), &self.schema)?;
        let mut source = Automerge::load(payload).map_err(|error| CrdtError::ImportRejected {
            detail: format!("automerge rejected state: {error}"),
        })?;
        validate_state(&source, &self.schema)?;
        self.doc
            .merge(&mut source)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("automerge merge failed: {error}"),
            })?;
        Ok(())
    }
}

fn validate_state(doc: &Automerge, schema: &DocumentSchema) -> Result<(), CrdtError> {
    for key in doc.keys(automerge::ROOT) {
        let Some((value, _)) =
            doc.get(automerge::ROOT, &key)
                .map_err(|error| CrdtError::ImportRejected {
                    detail: format!("automerge root read failed: {error}"),
                })?
        else {
            continue;
        };
        let valid = schema.iter().any(|(logical, kind)| match kind {
            FieldKind::Text => {
                storage_name(TEXT_PREFIX, logical) == key
                    && matches!(value, Value::Object(ObjType::Text))
            }
            FieldKind::Scalar => {
                storage_name(SCALAR_PREFIX, logical) == key
                    && matches!(value, Value::Scalar(ref scalar) if matches!(scalar.as_ref(), ScalarValue::Str(_)))
            }
        });
        if !valid {
            return Err(CrdtError::ImportRejected {
                detail: format!("unexpected automerge root field {key}"),
            });
        }
    }
    Ok(())
}

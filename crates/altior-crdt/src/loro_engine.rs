//! The Loro implementation of [`SyncDocumentEngine`] (ADR 0010).

use loro::{ExportMode, LoroDoc, LoroValue, ValueOrContainer};

use crate::{
    CrdtError, DocumentSchema, FieldKind, FieldView, SyncDocumentEngine, frame_state, parse_state,
    storage_name,
};

/// Scalar fields live in one root map so they never collide with the
/// root text containers.
const SCALAR_CONTAINER: &str = "__altior_crdt_v1_scalars__";
const TEXT_PREFIX: &str = "__altior_crdt_v1_text_";
const SCALAR_PREFIX: &str = "__altior_crdt_v1_scalar_";
const ENGINE_TAG: u8 = 1;

/// `SyncDocumentEngine` over a [`LoroDoc`].
#[derive(Debug)]
pub struct LoroEngine {
    doc: LoroDoc,
    schema: DocumentSchema,
}

impl LoroEngine {
    /// A fresh empty document. The peer id is fixed (not Loro's
    /// default random one) so encoded states are deterministic — the
    /// fork path assigns distinct ids per replica.
    ///
    /// # Panics
    ///
    /// Panics when Loro rejects the peer id assignment (an internal
    /// failure, not user input).
    #[must_use]
    pub fn new() -> Self {
        let doc = LoroDoc::new();
        doc.set_peer_id(1).expect("set peer id");
        Self {
            doc,
            schema: DocumentSchema::empty(),
        }
    }
}

impl Default for LoroEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl LoroEngine {
    /// Loro's root containers are name-addressed, so pre-declaring
    /// fields is unnecessary; kept for port parity.
    ///
    /// # Panics
    ///
    /// Panics when Loro rejects the peer id assignment (an internal
    /// failure, not user input).
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
        for (field, kind) in engine.schema.iter() {
            match kind {
                FieldKind::Text => {
                    engine.doc.get_text(storage_name(TEXT_PREFIX, field));
                }
                FieldKind::Scalar => {
                    engine.doc.get_map(SCALAR_CONTAINER);
                }
            }
        }
        Ok(engine)
    }

    /// Forks this document under a deterministic peer identity.
    ///
    /// # Panics
    ///
    /// Panics when Loro rejects the peer id assignment (an internal
    /// failure, not user input).
    #[must_use]
    pub fn fork_with_peer(&self, peer: u64) -> Self {
        let doc = self.doc.fork();
        doc.set_peer_id(peer).expect("set peer id");
        Self {
            doc,
            schema: self.schema.clone(),
        }
    }
}

impl SyncDocumentEngine for LoroEngine {
    fn engine_name(&self) -> &'static str {
        "loro"
    }

    fn insert_text(&mut self, field: &str, pos: usize, text: &str) -> Result<(), CrdtError> {
        self.schema.require(field, FieldKind::Text)?;
        let container = self.doc.get_text(storage_name(TEXT_PREFIX, field));
        let len = container.to_string().chars().count();
        let pos = pos.min(len);
        if !text.is_empty() {
            container
                .insert(pos, text)
                .map_err(|error| CrdtError::ImportRejected {
                    detail: format!("loro text insert failed: {error}"),
                })?;
        }
        Ok(())
    }

    fn delete_text(&mut self, field: &str, pos: usize, len: usize) -> Result<(), CrdtError> {
        self.schema.require(field, FieldKind::Text)?;
        let container = self.doc.get_text(storage_name(TEXT_PREFIX, field));
        let total = container.to_string().chars().count();
        let start = pos.min(total);
        let count = len.min(total - start);
        if count > 0 {
            container
                .delete(start, count)
                .map_err(|error| CrdtError::ImportRejected {
                    detail: format!("loro text delete failed: {error}"),
                })?;
        }
        Ok(())
    }

    fn set_field(&mut self, field: &str, value: &str) -> Result<(), CrdtError> {
        self.schema.require(field, FieldKind::Scalar)?;
        self.doc
            .get_map(SCALAR_CONTAINER)
            .insert(&storage_name(SCALAR_PREFIX, field), value)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("loro scalar insert failed: {error}"),
            })?;
        Ok(())
    }

    fn text_of(&self, field: &str) -> Result<String, CrdtError> {
        self.schema.require(field, FieldKind::Text)?;
        Ok(self
            .doc
            .get_text(storage_name(TEXT_PREFIX, field))
            .to_string())
    }

    fn field_of(&self, field: &str) -> Result<Option<String>, CrdtError> {
        self.schema.require(field, FieldKind::Scalar)?;
        Ok(
            match self
                .doc
                .get_map(SCALAR_CONTAINER)
                .get(&storage_name(SCALAR_PREFIX, field))
            {
                Some(ValueOrContainer::Value(LoroValue::String(value))) => Some(value.to_string()),
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
                    FieldView::Text(
                        self.doc
                            .get_text(storage_name(TEXT_PREFIX, name))
                            .to_string(),
                    ),
                ),
                FieldKind::Scalar => (
                    name.to_owned(),
                    FieldView::Scalar(self.field_of(name).ok().flatten().unwrap_or_default()),
                ),
            })
            .collect()
    }

    fn export_state(&self) -> Vec<u8> {
        let payload = self.doc.export(ExportMode::Snapshot).expect("loro export");
        frame_state(ENGINE_TAG, &self.schema, &payload)
    }

    fn import_state(&mut self, bytes: &[u8]) -> Result<(), CrdtError> {
        let payload = parse_state(bytes, ENGINE_TAG, self.engine_name(), &self.schema)?;
        let probe = LoroDoc::new();
        probe
            .import(payload)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("loro rejected snapshot: {error}"),
            })?;
        validate_state(&probe, &self.schema)?;
        self.doc
            .import(payload)
            .map_err(|error| CrdtError::ImportRejected {
                detail: format!("loro rejected snapshot merge: {error}"),
            })?;
        Ok(())
    }
}

fn validate_state(doc: &LoroDoc, schema: &DocumentSchema) -> Result<(), CrdtError> {
    let LoroValue::Map(root) = doc.get_deep_value() else {
        return Err(CrdtError::ImportRejected {
            detail: "loro root is not a map".to_owned(),
        });
    };
    for (name, value) in root.iter() {
        if name == SCALAR_CONTAINER {
            let LoroValue::Map(values) = value else {
                return Err(CrdtError::ImportRejected {
                    detail: "scalar namespace is not a map".to_owned(),
                });
            };
            for (stored, scalar) in values.iter() {
                let allowed = schema.iter().any(|(logical, kind)| {
                    kind == FieldKind::Scalar && storage_name(SCALAR_PREFIX, logical) == *stored
                });
                if !allowed || !matches!(scalar, LoroValue::String(_)) {
                    return Err(CrdtError::ImportRejected {
                        detail: format!("unexpected scalar storage field {stored}"),
                    });
                }
            }
            continue;
        }
        let allowed = schema.iter().any(|(logical, kind)| {
            kind == FieldKind::Text && storage_name(TEXT_PREFIX, logical) == *name
        });
        if !allowed || !matches!(value, LoroValue::String(_)) {
            return Err(CrdtError::ImportRejected {
                detail: format!("unexpected loro root field {name}"),
            });
        }
    }
    Ok(())
}

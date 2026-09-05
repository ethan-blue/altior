//! Device-local identity documents and `ContextSnapshot` audit storage (P2.2, ADR 0018).
//!
//! Identity documents and context snapshots are device-local records:
//! they never append to the authoritative `domain_journal`, never sync
//! across devices in P2, and are completely isolated from domain
//! projection digests and rebuilds.

use rusqlite::{OptionalExtension, params};

use altior_domain::{
    CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES, ContextSnapshot, ContextSnapshotListLimit,
    IDENTITY_DOCUMENT_COUNT_MAX, IdentityContent, IdentityDocument, IdentityDocumentId,
    IdentityDocumentKind, IdentityDocumentListLimit, ThreadId, TurnId, UnixMillis,
    is_secret_shaped,
};

use crate::error::StorageError;
use crate::{Store, collect_rows};

/// Maps one SQLite row to a validated [`IdentityDocument`].
fn row_to_identity_document(row: &rusqlite::Row<'_>) -> Result<IdentityDocument, StorageError> {
    let invalid = |detail: String| StorageError::InvalidEntityData { detail };
    let id_raw: String = row
        .get(0)
        .map_err(|e| StorageError::from_sqlite("read identity_document_id", e))?;
    let kind_raw: String = row
        .get(1)
        .map_err(|e| StorageError::from_sqlite("read identity kind", e))?;
    let content_raw: String = row
        .get(2)
        .map_err(|e| StorageError::from_sqlite("read identity content", e))?;
    let created_at_raw: i64 = row
        .get(3)
        .map_err(|e| StorageError::from_sqlite("read identity created_at", e))?;
    let updated_at_raw: i64 = row
        .get(4)
        .map_err(|e| StorageError::from_sqlite("read identity updated_at", e))?;

    let id = id_raw
        .parse::<IdentityDocumentId>()
        .map_err(|e| invalid(format!("invalid identity_document_id {id_raw}: {e}")))?;
    let kind = IdentityDocumentKind::try_from_str(&kind_raw)
        .map_err(|e| invalid(format!("invalid identity kind {kind_raw}: {e}")))?;
    let content = IdentityContent::try_from(content_raw.as_str())
        .map_err(|e| invalid(format!("invalid identity content: {e}")))?;
    let created_at = u64::try_from(created_at_raw)
        .map(UnixMillis::from_millis)
        .map_err(|_| invalid(format!("negative created_at {created_at_raw}")))?;
    let updated_at = u64::try_from(updated_at_raw)
        .map(UnixMillis::from_millis)
        .map_err(|_| invalid(format!("negative updated_at {updated_at_raw}")))?;

    let doc = IdentityDocument {
        id,
        kind,
        content,
        created_at,
        updated_at,
    };
    doc.validate()
        .map_err(|e| invalid(format!("identity document invariant violated: {e}")))?;
    Ok(doc)
}

/// Validates individual string fields inside a `ContextSnapshot` for secret shapes.
fn validate_snapshot_secrets(snapshot: &ContextSnapshot) -> Result<(), StorageError> {
    for mem in &snapshot.memories {
        if is_secret_shaped(&mem.why_selected) {
            return Err(StorageError::SecretShapedContent);
        }
    }
    if let Some(ref deg) = snapshot.degraded
        && (is_secret_shaped(&deg.code) || is_secret_shaped(&deg.detail))
    {
        return Err(StorageError::SecretShapedContent);
    }
    if let Some(ref prompt) = snapshot.rendered_prompt
        && is_secret_shaped(prompt)
    {
        return Err(StorageError::SecretShapedContent);
    }
    Ok(())
}

impl Store {
    /// Inserts an identity document idempotently.
    ///
    /// If an identity document with the same ID already exists:
    /// - If the kind and content are identical, this succeeds idempotently.
    /// - If the kind or content differs, returns [`StorageError::IdentityDocumentConflict`].
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::SecretShapedContent`] if content is secret-shaped (fail-closed),
    /// [`StorageError::IdentityDocumentCountExceeded`] if device capacity (32) is reached,
    /// [`StorageError::InvalidEntityData`] if timestamps or bounds are invalid,
    /// or [`StorageError::IdentityDocumentConflict`] if conflicting content exists.
    pub fn put_identity_document(
        &mut self,
        document: &IdentityDocument,
    ) -> Result<(), StorageError> {
        if is_secret_shaped(document.content.as_str()) {
            return Err(StorageError::SecretShapedContent);
        }

        document
            .validate()
            .map_err(|e| StorageError::InvalidEntityData {
                detail: e.to_string(),
            })?;

        let created_at = i64::try_from(document.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", document.created_at.as_millis()),
            }
        })?;
        let updated_at = i64::try_from(document.updated_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("updated_at {} exceeds i64", document.updated_at.as_millis()),
            }
        })?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StorageError::from_sqlite("put_identity_document begin tx", e))?;

        let mut check_stmt = tx
            .prepare("SELECT kind, content FROM identity_document WHERE identity_document_id = ?1")
            .map_err(|e| StorageError::from_sqlite("put_identity_document check stmt", e))?;

        let existing: Option<(String, String)> = check_stmt
            .query_row(params![document.id.as_str()], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })
            .optional()
            .map_err(|e| StorageError::from_sqlite("put_identity_document query existing", e))?;
        drop(check_stmt);

        if let Some((existing_kind, existing_content)) = existing {
            if existing_kind == document.kind.as_str()
                && existing_content == document.content.as_str()
            {
                return Ok(());
            }
            return Err(StorageError::IdentityDocumentConflict {
                identity_document_id: document.id.to_string(),
            });
        }

        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM identity_document", [], |r| r.get(0))
            .map_err(|e| StorageError::from_sqlite("put_identity_document count", e))?;

        let count_usize = usize::try_from(count).unwrap_or(usize::MAX);
        if count_usize >= IDENTITY_DOCUMENT_COUNT_MAX {
            return Err(StorageError::IdentityDocumentCountExceeded {
                count: count_usize,
                max: IDENTITY_DOCUMENT_COUNT_MAX,
            });
        }

        tx.execute(
            "INSERT INTO identity_document (identity_document_id, kind, content, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                document.id.as_str(),
                document.kind.as_str(),
                document.content.as_str(),
                created_at,
                updated_at,
            ],
        )
        .map_err(|e| StorageError::from_sqlite("put_identity_document insert", e))?;

        tx.commit()
            .map_err(|e| StorageError::from_sqlite("put_identity_document commit", e))?;
        Ok(())
    }

    /// Creates an identity document, delegating to [`Self::put_identity_document`].
    ///
    /// # Errors
    ///
    /// See [`Self::put_identity_document`].
    pub fn create_identity_document(
        &mut self,
        document: &IdentityDocument,
    ) -> Result<(), StorageError> {
        self.put_identity_document(document)
    }

    /// Fetches an identity document by its unique ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::InvalidEntityData`] if stored data violates domain invariants,
    /// or [`StorageError::Sqlite`] on database errors.
    pub fn get_identity_document(
        &self,
        id: &IdentityDocumentId,
    ) -> Result<Option<IdentityDocument>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT identity_document_id, kind, content, created_at, updated_at
                 FROM identity_document WHERE identity_document_id = ?1",
            )
            .map_err(|e| StorageError::from_sqlite("get_identity_document prepare", e))?;
        let mut rows = stmt
            .query(params![id.as_str()])
            .map_err(|e| StorageError::from_sqlite("get_identity_document query", e))?;
        if let Some(row) = rows
            .next()
            .map_err(|e| StorageError::from_sqlite("get_identity_document fetch", e))?
        {
            Ok(Some(row_to_identity_document(row)?))
        } else {
            Ok(None)
        }
    }

    /// Alias for [`Self::get_identity_document`].
    ///
    /// # Errors
    ///
    /// See [`Self::get_identity_document`].
    pub fn identity_document_by_id(
        &self,
        id: &IdentityDocumentId,
    ) -> Result<Option<IdentityDocument>, StorageError> {
        self.get_identity_document(id)
    }

    /// Updates an existing identity document.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::IdentityDocumentNotFound`] if the document does not exist,
    /// [`StorageError::SecretShapedContent`] if new content is secret-shaped,
    /// or [`StorageError::InvalidEntityData`] if domain invariants are violated.
    pub fn update_identity_document(
        &mut self,
        document: &IdentityDocument,
    ) -> Result<(), StorageError> {
        document
            .validate()
            .map_err(|e| StorageError::InvalidEntityData {
                detail: e.to_string(),
            })?;

        if is_secret_shaped(document.content.as_str()) {
            return Err(StorageError::SecretShapedContent);
        }

        let updated_at = i64::try_from(document.updated_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("updated_at {} exceeds i64", document.updated_at.as_millis()),
            }
        })?;

        let affected = self
            .conn
            .execute(
                "UPDATE identity_document
                 SET kind = ?1, content = ?2, updated_at = ?3
                 WHERE identity_document_id = ?4",
                params![
                    document.kind.as_str(),
                    document.content.as_str(),
                    updated_at,
                    document.id.as_str(),
                ],
            )
            .map_err(|e| StorageError::from_sqlite("update_identity_document", e))?;

        if affected == 0 {
            return Err(StorageError::IdentityDocumentNotFound {
                identity_document_id: document.id.to_string(),
            });
        }
        Ok(())
    }

    /// Deletes an identity document by ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::IdentityDocumentNotFound`] if the document does not exist,
    /// or [`StorageError::Sqlite`] on database errors.
    pub fn delete_identity_document(
        &mut self,
        id: &IdentityDocumentId,
    ) -> Result<(), StorageError> {
        let affected = self
            .conn
            .execute(
                "DELETE FROM identity_document WHERE identity_document_id = ?1",
                params![id.as_str()],
            )
            .map_err(|e| StorageError::from_sqlite("delete_identity_document", e))?;

        if affected == 0 {
            return Err(StorageError::IdentityDocumentNotFound {
                identity_document_id: id.to_string(),
            });
        }
        Ok(())
    }

    /// Lists identity documents in render-priority order.
    ///
    /// Ordering is: `Name` (0) < `About` (1) < `Instruction` (2) < `Preference` (3),
    /// tie-broken deterministically by `created_at` ASC, then `identity_document_id` ASC.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on query failure.
    pub fn list_identity_documents(
        &self,
        limit: IdentityDocumentListLimit,
    ) -> Result<Vec<IdentityDocument>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT identity_document_id, kind, content, created_at, updated_at
                 FROM identity_document
                 ORDER BY
                     CASE kind
                         WHEN 'name' THEN 0
                         WHEN 'about' THEN 1
                         WHEN 'instruction' THEN 2
                         WHEN 'preference' THEN 3
                         ELSE 4
                     END ASC,
                     created_at ASC,
                     identity_document_id ASC
                 LIMIT ?1",
            )
            .map_err(|e| StorageError::from_sqlite("list_identity_documents prepare", e))?;

        let rows = stmt
            .query(params![i64::from(limit.get())])
            .map_err(|e| StorageError::from_sqlite("list_identity_documents query", e))?;

        collect_rows(rows, row_to_identity_document, "list_identity_documents")
    }

    /// Alias for [`Self::list_identity_documents`].
    ///
    /// # Errors
    ///
    /// See [`Self::list_identity_documents`].
    pub fn identity_documents(
        &self,
        limit: IdentityDocumentListLimit,
    ) -> Result<Vec<IdentityDocument>, StorageError> {
        self.list_identity_documents(limit)
    }

    /// Returns the total count of stored identity documents on this device.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::Sqlite`] on failure.
    pub fn count_identity_documents(&self) -> Result<usize, StorageError> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM identity_document", [], |r| r.get(0))
            .map_err(|e| StorageError::from_sqlite("count_identity_documents", e))?;
        Ok(usize::try_from(count).unwrap_or(0))
    }

    /// Records an immutable audit snapshot for a turn's assembled context.
    ///
    /// Idempotency contract:
    /// - If a snapshot for `turn_id` already exists with the same content, succeeds idempotently.
    /// - If a snapshot for `turn_id` already exists with different content, returns [`StorageError::ContextSnapshotConflict`].
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::SecretShapedContent`] if any component or serialized payload is secret-shaped,
    /// [`StorageError::PayloadTooLarge`] if serialized JSON exceeds 64 KiB,
    /// [`StorageError::ContextSnapshotConflict`] if conflicting snapshot exists for `turn_id`,
    /// or [`StorageError::Sqlite`] on database error.
    pub fn record_context_snapshot(
        &mut self,
        snapshot: &ContextSnapshot,
    ) -> Result<(), StorageError> {
        validate_snapshot_secrets(snapshot)?;

        let payload_json =
            serde_json::to_string(snapshot).map_err(|e| StorageError::InvalidEntityData {
                detail: format!("failed to serialize ContextSnapshot to JSON: {e}"),
            })?;

        if is_secret_shaped(&payload_json) {
            return Err(StorageError::SecretShapedContent);
        }

        if payload_json.len() > CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES {
            return Err(StorageError::PayloadTooLarge {
                size_bytes: payload_json.len(),
                limit_bytes: CONTEXT_SNAPSHOT_PAYLOAD_MAX_BYTES,
            });
        }

        let created_at = i64::try_from(snapshot.created_at.as_millis()).map_err(|_| {
            StorageError::InvalidEntityData {
                detail: format!("created_at {} exceeds i64", snapshot.created_at.as_millis()),
            }
        })?;

        let tx = self
            .conn
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
            .map_err(|e| StorageError::from_sqlite("record_context_snapshot begin tx", e))?;

        let mut check_stmt = tx
            .prepare("SELECT payload_json FROM context_snapshot WHERE turn_id = ?1")
            .map_err(|e| StorageError::from_sqlite("record_context_snapshot check stmt", e))?;

        let existing_json: Option<String> = check_stmt
            .query_row(params![snapshot.turn_id.as_str()], |r| r.get(0))
            .optional()
            .map_err(|e| StorageError::from_sqlite("record_context_snapshot query existing", e))?;
        drop(check_stmt);

        if let Some(existing) = existing_json {
            if existing == payload_json {
                return Ok(());
            }
            if let Ok(existing_snap) = serde_json::from_str::<ContextSnapshot>(&existing)
                && existing_snap == *snapshot
            {
                return Ok(());
            }
            return Err(StorageError::ContextSnapshotConflict {
                turn_id: snapshot.turn_id.to_string(),
            });
        }

        let passthrough_val = i64::from(snapshot.passthrough);

        tx.execute(
            "INSERT INTO context_snapshot
                 (turn_id, thread_id, memory_mode, passthrough, created_at, payload_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                snapshot.turn_id.as_str(),
                snapshot.thread_id.as_str(),
                snapshot.memory_mode.as_str(),
                passthrough_val,
                created_at,
                payload_json,
            ],
        )
        .map_err(|e| StorageError::from_sqlite("record_context_snapshot insert", e))?;

        tx.commit()
            .map_err(|e| StorageError::from_sqlite("record_context_snapshot commit", e))?;
        Ok(())
    }

    /// Alias for [`Self::record_context_snapshot`].
    ///
    /// # Errors
    ///
    /// See [`Self::record_context_snapshot`].
    pub fn persist_context_snapshot(
        &mut self,
        snapshot: &ContextSnapshot,
    ) -> Result<(), StorageError> {
        self.record_context_snapshot(snapshot)
    }

    /// Fetches an audit snapshot by turn ID.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ContextSnapshotCorrupt`] if the stored JSON cannot be parsed,
    /// or [`StorageError::Sqlite`] on database error.
    pub fn get_context_snapshot(
        &self,
        turn_id: &TurnId,
    ) -> Result<Option<ContextSnapshot>, StorageError> {
        let mut stmt = self
            .conn
            .prepare("SELECT payload_json FROM context_snapshot WHERE turn_id = ?1")
            .map_err(|e| StorageError::from_sqlite("get_context_snapshot prepare", e))?;

        let mut rows = stmt
            .query(params![turn_id.as_str()])
            .map_err(|e| StorageError::from_sqlite("get_context_snapshot query", e))?;

        if let Some(row) = rows
            .next()
            .map_err(|e| StorageError::from_sqlite("get_context_snapshot fetch", e))?
        {
            let payload_json: String = row
                .get(0)
                .map_err(|e| StorageError::from_sqlite("read payload_json", e))?;
            let snapshot = serde_json::from_str::<ContextSnapshot>(&payload_json).map_err(|e| {
                StorageError::ContextSnapshotCorrupt {
                    turn_id: turn_id.to_string(),
                    detail: e.to_string(),
                }
            })?;
            Ok(Some(snapshot))
        } else {
            Ok(None)
        }
    }

    /// Alias for [`Self::get_context_snapshot`].
    ///
    /// # Errors
    ///
    /// See [`Self::get_context_snapshot`].
    pub fn context_snapshot_by_turn_id(
        &self,
        turn_id: &TurnId,
    ) -> Result<Option<ContextSnapshot>, StorageError> {
        self.get_context_snapshot(turn_id)
    }

    /// Lists context snapshots for a specific thread, ordered by `created_at` DESC, `turn_id` DESC.
    ///
    /// # Errors
    ///
    /// Returns [`StorageError::ContextSnapshotCorrupt`] if any stored row contains invalid JSON,
    /// or [`StorageError::Sqlite`] on database error.
    pub fn list_context_snapshots_for_thread(
        &self,
        thread_id: &ThreadId,
        limit: ContextSnapshotListLimit,
    ) -> Result<Vec<ContextSnapshot>, StorageError> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT turn_id, payload_json
                 FROM context_snapshot
                 WHERE thread_id = ?1
                 ORDER BY created_at DESC, turn_id DESC
                 LIMIT ?2",
            )
            .map_err(|e| {
                StorageError::from_sqlite("list_context_snapshots_for_thread prepare", e)
            })?;

        let mut rows = stmt
            .query(params![thread_id.as_str(), i64::from(limit.get())])
            .map_err(|e| StorageError::from_sqlite("list_context_snapshots_for_thread query", e))?;

        let mut result = Vec::new();
        while let Some(row) = rows
            .next()
            .map_err(|e| StorageError::from_sqlite("list_context_snapshots_for_thread next", e))?
        {
            let turn_id_raw: String = row
                .get(0)
                .map_err(|e| StorageError::from_sqlite("read turn_id", e))?;
            let payload_json: String = row
                .get(1)
                .map_err(|e| StorageError::from_sqlite("read payload_json", e))?;
            let snap = serde_json::from_str::<ContextSnapshot>(&payload_json).map_err(|e| {
                StorageError::ContextSnapshotCorrupt {
                    turn_id: turn_id_raw,
                    detail: e.to_string(),
                }
            })?;
            result.push(snap);
        }
        Ok(result)
    }

    /// Alias for [`Self::list_context_snapshots_for_thread`].
    ///
    /// # Errors
    ///
    /// See [`Self::list_context_snapshots_for_thread`].
    pub fn context_snapshots_for_thread(
        &self,
        thread_id: &ThreadId,
        limit: ContextSnapshotListLimit,
    ) -> Result<Vec<ContextSnapshot>, StorageError> {
        self.list_context_snapshots_for_thread(thread_id, limit)
    }
}

//! JSON snapshot writer, reader, path resolver, and schema hashing.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::model::{ColumnProfile, DbEdge, DbObject, DbSnapshot, TableProfile};
use crate::project::ProjectContext;
use crate::{DbGraphError, Result};

/// Snapshot JSON storage helper.
#[derive(Debug, Clone)]
pub struct SnapshotStore {
    snapshots_dir: PathBuf,
}

impl SnapshotStore {
    /// Creates a store from project context.
    #[must_use]
    pub fn new(context: &ProjectContext) -> Self {
        Self {
            snapshots_dir: context.snapshots_dir(),
        }
    }

    /// Creates a store from a raw snapshots directory.
    #[must_use]
    pub fn from_dir(snapshots_dir: impl Into<PathBuf>) -> Self {
        Self {
            snapshots_dir: snapshots_dir.into(),
        }
    }

    /// Returns the snapshots directory.
    #[must_use]
    pub fn snapshots_dir(&self) -> &Path {
        &self.snapshots_dir
    }

    /// Writes a snapshot and returns the created path.
    ///
    /// # Errors
    ///
    /// Returns an error when hashing, serialization, or filesystem writes fail.
    pub fn write_snapshot(&self, snapshot: &DbSnapshot, pretty: bool) -> Result<PathBuf> {
        fs::create_dir_all(&self.snapshots_dir)
            .map_err(|source| DbGraphError::io(&self.snapshots_dir, source))?;

        let mut snapshot = snapshot.clone();
        let schema_hash = compute_schema_hash(&snapshot)?;
        snapshot.schema_hash = Some(schema_hash.clone());
        let path = self.snapshot_path(snapshot.created_at_unix_ms, &schema_hash);
        let content = if pretty {
            serde_json::to_string_pretty(&snapshot)
        } else {
            serde_json::to_string(&snapshot)
        }
        .map_err(|source| DbGraphError::Internal {
            message: format!("failed to serialize snapshot: {source}"),
        })?;
        fs::write(&path, format!("{content}\n"))
            .map_err(|source| DbGraphError::io(&path, source))?;
        Ok(path)
    }

    /// Reads a snapshot from a path.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be read or contains invalid JSON.
    pub fn read_snapshot(&self, path: impl AsRef<Path>) -> Result<DbSnapshot> {
        let path = path.as_ref();
        let content = fs::read_to_string(path).map_err(|source| DbGraphError::io(path, source))?;
        serde_json::from_str(&content).map_err(|source| {
            DbGraphError::invalid_config(format!(
                "failed to parse snapshot {}: {source}",
                path.display()
            ))
        })
    }

    /// Reads the latest snapshot if present.
    ///
    /// # Errors
    ///
    /// Returns an error when the directory cannot be read or the latest file is invalid.
    pub fn read_latest(&self) -> Result<Option<DbSnapshot>> {
        self.latest_snapshot_path()?
            .map(|path| self.read_snapshot(path))
            .transpose()
    }

    /// Returns the latest snapshot path.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshots directory cannot be read.
    pub fn latest_snapshot_path(&self) -> Result<Option<PathBuf>> {
        Ok(self.snapshot_paths()?.pop())
    }

    /// Returns the previous snapshot path.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshots directory cannot be read.
    pub fn previous_snapshot_path(&self) -> Result<Option<PathBuf>> {
        let paths = self.snapshot_paths()?;
        Ok(paths.get(paths.len().saturating_sub(2)).cloned())
    }

    /// Lists snapshot paths in ascending file-name order.
    ///
    /// # Errors
    ///
    /// Returns an error when the snapshots directory cannot be read.
    pub fn snapshot_paths(&self) -> Result<Vec<PathBuf>> {
        if !self.snapshots_dir.is_dir() {
            return Ok(Vec::new());
        }

        let mut paths = fs::read_dir(&self.snapshots_dir)
            .map_err(|source| DbGraphError::io(&self.snapshots_dir, source))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .filter(|path| {
                path.extension()
                    .is_some_and(|extension| extension == "json")
            })
            .collect::<Vec<_>>();
        paths.sort();
        Ok(paths)
    }

    fn snapshot_path(&self, created_at_unix_ms: u64, schema_hash: &str) -> PathBuf {
        let hash_prefix = schema_hash.chars().take(12).collect::<String>();
        self.snapshots_dir
            .join(format!("snapshot-{created_at_unix_ms}-{hash_prefix}.json"))
    }
}

/// Current Unix timestamp in milliseconds.
///
/// # Errors
///
/// Returns an internal error if the system clock is before the Unix epoch.
pub fn now_unix_ms() -> Result<u64> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|source| DbGraphError::Internal {
            message: format!("system clock is before Unix epoch: {source}"),
        })?;
    Ok(u64::try_from(duration.as_millis()).unwrap_or(u64::MAX))
}

/// Computes a deterministic schema hash for a snapshot.
///
/// # Errors
///
/// Returns an error if canonical serialization fails.
pub fn compute_schema_hash(snapshot: &DbSnapshot) -> Result<String> {
    let canonical = CanonicalSnapshot::from(snapshot);
    let bytes = serde_json::to_vec(&canonical).map_err(|source| DbGraphError::Internal {
        message: format!("failed to serialize canonical snapshot: {source}"),
    })?;
    let digest = Sha256::digest(bytes);
    Ok(hex_encode(&digest))
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(char::from(HEX[usize::from(byte >> 4)]));
        out.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CanonicalSnapshot {
    provider: String,
    database_name: String,
    objects: Vec<DbObject>,
    edges: Vec<DbEdge>,
    table_profiles: Vec<TableProfile>,
    column_profiles: Vec<ColumnProfile>,
}

impl From<&DbSnapshot> for CanonicalSnapshot {
    fn from(snapshot: &DbSnapshot) -> Self {
        let mut objects = snapshot.objects.clone();
        objects.sort_by(|left, right| {
            left.kind
                .as_str()
                .cmp(right.kind.as_str())
                .then_with(|| left.full_name.cmp(&right.full_name))
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut edges = snapshot.edges.clone();
        edges.sort_by(|left, right| {
            left.kind
                .as_str()
                .cmp(right.kind.as_str())
                .then_with(|| left.from_object_id.cmp(&right.from_object_id))
                .then_with(|| left.to_object_id.cmp(&right.to_object_id))
                .then_with(|| left.id.cmp(&right.id))
        });

        let mut table_profiles = snapshot.table_profiles.clone();
        table_profiles.sort_by(|left, right| left.object_id.cmp(&right.object_id));

        let mut column_profiles = snapshot.column_profiles.clone();
        column_profiles.sort_by(|left, right| left.object_id.cmp(&right.object_id));

        Self {
            provider: snapshot.provider.clone(),
            database_name: snapshot.database_name.clone(),
            objects,
            edges,
            table_profiles,
            column_profiles,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ColumnMetadata, DbEdgeKind, DbObjectKind};

    #[test]
    fn same_schema_has_same_hash_even_when_order_changes() {
        let mut left = sample_snapshot();
        let mut right = sample_snapshot();
        right.objects.reverse();
        right.edges.reverse();
        left.created_at_unix_ms = 1;
        right.created_at_unix_ms = 2;

        let left_hash = compute_schema_hash(&left).expect("hash should compute");
        let right_hash = compute_schema_hash(&right).expect("hash should compute");

        assert_eq!(left_hash, right_hash);
    }

    #[test]
    fn write_and_read_latest_snapshot() {
        let temp = TempDir::new();
        let store = SnapshotStore::from_dir(&temp.path);
        let snapshot = sample_snapshot();

        let path = store
            .write_snapshot(&snapshot, true)
            .expect("snapshot should write");
        let latest = store.read_latest().expect("latest should read");

        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with("snapshot-"));
        assert_eq!(
            latest.unwrap().schema_hash,
            Some(compute_schema_hash(&snapshot).unwrap())
        );
    }

    #[test]
    fn corrupted_json_reports_clear_error() {
        let temp = TempDir::new();
        let path = temp.path.join("bad.json");
        fs::write(&path, "{ not json").expect("bad json should be written");
        let store = SnapshotStore::from_dir(&temp.path);

        let err = store
            .read_snapshot(&path)
            .expect_err("corrupted snapshot should fail");

        assert!(err.to_string().contains("failed to parse snapshot"));
    }

    fn sample_snapshot() -> DbSnapshot {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        let mut table = DbObject::new("table:public.users", DbObjectKind::Table, "public.users");
        table.schema_name = Some("public".to_owned());
        table.table_name = Some("users".to_owned());
        let mut column = DbObject::new(
            "column:public.users.id",
            DbObjectKind::Column,
            "public.users.id",
        );
        column.schema_name = Some("public".to_owned());
        column.table_name = Some("users".to_owned());
        column.column_name = Some("id".to_owned());
        column.column = Some(ColumnMetadata {
            data_type: Some("bigint".to_owned()),
            data_type_family: Some("integer".to_owned()),
            nullable: Some(false),
            default: None,
            comment: None,
        });
        snapshot.objects = vec![table, column];
        snapshot.edges = vec![DbEdge::explicit(
            "edge:users:id",
            DbEdgeKind::HasColumn,
            "table:public.users",
            "column:public.users.id",
        )];
        snapshot
    }

    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "dbgraph-snapshot-test-{}-{}",
                std::process::id(),
                now_unix_ms().expect("time should be valid")
            ));
            fs::create_dir_all(&path).expect("temp dir should be created");
            Self { path }
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            if self.path.exists() {
                let _ = fs::remove_dir_all(&self.path);
            }
        }
    }
}

//! Local `SQLite` storage for `DbGraph` snapshots and graph index.

use std::path::Path;

use dbgraph_core::model::{
    ColumnProfile, DbEdge, DbObject, DbObjectKind, DbSnapshot, TableProfile,
};
use dbgraph_core::{DbGraphError, Result};
use rusqlite::{params, Connection, OptionalExtension, Transaction};

/// Current storage schema version.
pub const CURRENT_SCHEMA_VERSION: i64 = 1;

/// Local graph database repository.
pub struct GraphRepository {
    conn: Connection,
    fts_enabled: bool,
}

impl GraphRepository {
    /// Opens or creates a graph database and runs migrations.
    ///
    /// # Errors
    ///
    /// Returns an error if the database cannot be opened or migrated.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path).map_err(sql_error)?;
        let fts_enabled = run_migrations(&conn)?;
        Ok(Self { conn, fts_enabled })
    }

    /// Opens an in-memory repository for tests.
    ///
    /// # Errors
    ///
    /// Returns an error if migrations fail.
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory().map_err(|source| DbGraphError::Internal {
            message: format!("failed to open in-memory SQLite database: {source}"),
        })?;
        let fts_enabled = run_migrations(&conn)?;
        Ok(Self { conn, fts_enabled })
    }

    /// Returns whether `FTS5` was created.
    #[must_use]
    pub const fn fts_enabled(&self) -> bool {
        self.fts_enabled
    }

    /// Returns the current schema version.
    ///
    /// # Errors
    ///
    /// Returns an error if the version table cannot be queried.
    pub fn schema_version(&self) -> Result<i64> {
        schema_version(&self.conn)
    }

    /// Rebuilds all index rows for a snapshot in one transaction.
    ///
    /// # Errors
    ///
    /// Returns an error if any insert fails. The transaction rolls back.
    pub fn rebuild_snapshot(&mut self, snapshot: &DbSnapshot) -> Result<()> {
        let tx = self.conn.transaction().map_err(sql_error)?;
        delete_snapshot_index(&tx, &snapshot.id)?;
        insert_snapshot_tx(&tx, snapshot)?;
        insert_objects_tx(&tx, &snapshot.id, &snapshot.objects, self.fts_enabled)?;
        insert_edges_tx(&tx, &snapshot.id, &snapshot.edges)?;
        insert_table_profiles_tx(&tx, &snapshot.id, &snapshot.table_profiles)?;
        insert_column_profiles_tx(&tx, &snapshot.id, &snapshot.column_profiles)?;
        tx.commit().map_err(sql_error)
    }

    /// Inserts only the snapshot row.
    ///
    /// # Errors
    ///
    /// Returns an error if the row cannot be inserted.
    pub fn insert_snapshot(&self, snapshot: &DbSnapshot) -> Result<()> {
        insert_snapshot_conn(&self.conn, snapshot)
    }

    /// Inserts SQL artifacts, replacing prior rows with the same snapshot fingerprint.
    ///
    /// # Errors
    ///
    /// Returns an error if any artifact row cannot be written.
    pub fn insert_sql_artifacts(&mut self, artifacts: &[SqlArtifactRecord]) -> Result<()> {
        let tx = self.conn.transaction().map_err(sql_error)?;
        let mut stmt = tx
            .prepare(
                "INSERT OR REPLACE INTO sql_artifacts(
                    id, snapshot_id, source_kind, source_path, dialect, fingerprint,
                    normalized_sql, ast_json, analysis_json
                 )
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            )
            .map_err(sql_error)?;
        for artifact in artifacts {
            stmt.execute(params![
                artifact.id,
                artifact.snapshot_id,
                artifact.source_kind,
                artifact.source_path,
                artifact.dialect,
                artifact.fingerprint,
                artifact.normalized_sql,
                artifact.ast_json,
                artifact.analysis_json
            ])
            .map_err(sql_error)?;
        }
        drop(stmt);
        tx.commit().map_err(sql_error)
    }

    /// Lists SQL artifacts for a snapshot in stable source order.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn sql_artifacts_by_snapshot(&self, snapshot_id: &str) -> Result<Vec<SqlArtifactRecord>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, snapshot_id, source_kind, source_path, dialect, fingerprint,
                        normalized_sql, ast_json, analysis_json
                 FROM sql_artifacts
                 WHERE snapshot_id = ?1
                 ORDER BY source_path, fingerprint, id",
            )
            .map_err(sql_error)?;
        let rows = stmt
            .query_map(params![snapshot_id], row_to_sql_artifact)
            .map_err(sql_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)
    }

    /// Finds an object by full name and kind.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn find_object(
        &self,
        snapshot_id: &str,
        full_name: &str,
        kind: DbObjectKind,
    ) -> Result<Option<StoredObject>> {
        self.conn
            .query_row(
                "SELECT id, snapshot_id, kind, name, full_name, schema_name, table_name, column_name, metadata_json
                 FROM objects
                 WHERE snapshot_id = ?1 AND full_name = ?2 AND kind = ?3
                 ORDER BY full_name, id
                 LIMIT 1",
                params![snapshot_id, full_name, kind.as_str()],
                row_to_object,
            )
            .optional()
            .map_err(sql_error)
    }

    /// Lists objects for a snapshot in stable order.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn objects_by_snapshot(&self, snapshot_id: &str) -> Result<Vec<StoredObject>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, snapshot_id, kind, name, full_name, schema_name, table_name, column_name, metadata_json
                 FROM objects
                 WHERE snapshot_id = ?1
                 ORDER BY kind, full_name, id",
            )
            .map_err(sql_error)?;
        let rows = stmt
            .query_map(params![snapshot_id], row_to_object)
            .map_err(sql_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)
    }

    /// Lists edges for a snapshot in stable order.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn edges_by_snapshot(&self, snapshot_id: &str) -> Result<Vec<StoredEdge>> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, snapshot_id, kind, from_object_id, to_object_id, confidence, metadata_json
                 FROM edges
                 WHERE snapshot_id = ?1
                 ORDER BY kind, from_object_id, to_object_id, id",
            )
            .map_err(sql_error)?;
        let rows = stmt
            .query_map(params![snapshot_id], row_to_edge)
            .map_err(sql_error)?;
        rows.collect::<rusqlite::Result<Vec<_>>>()
            .map_err(sql_error)
    }

    /// Counts objects for a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn object_count(&self, snapshot_id: &str) -> Result<i64> {
        count_by_snapshot(&self.conn, "objects", snapshot_id)
    }

    /// Counts edges for a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn edge_count(&self, snapshot_id: &str) -> Result<i64> {
        count_by_snapshot(&self.conn, "edges", snapshot_id)
    }

    /// Searches object FTS content.
    ///
    /// # Errors
    ///
    /// Returns an error if the query fails.
    pub fn search_objects(&self, query: &str) -> Result<Vec<StoredObject>> {
        if self.fts_enabled {
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT o.id, o.snapshot_id, o.kind, o.name, o.full_name, o.schema_name, o.table_name, o.column_name, o.metadata_json
                     FROM object_fts f
                     JOIN objects o ON o.id = f.object_id AND o.snapshot_id = f.snapshot_id
                     WHERE object_fts MATCH ?1
                     ORDER BY o.full_name, o.id",
                )
                .map_err(sql_error)?;
            let rows = stmt
                .query_map(params![query], row_to_object)
                .map_err(sql_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)
        } else {
            let pattern = format!("%{query}%");
            let mut stmt = self
                .conn
                .prepare(
                    "SELECT id, snapshot_id, kind, name, full_name, schema_name, table_name, column_name, metadata_json
                     FROM objects
                     WHERE name LIKE ?1 OR full_name LIKE ?1 OR metadata_json LIKE ?1
                     ORDER BY full_name, id",
                )
                .map_err(sql_error)?;
            let rows = stmt
                .query_map(params![pattern], row_to_object)
                .map_err(sql_error)?;
            rows.collect::<rusqlite::Result<Vec<_>>>()
                .map_err(sql_error)
        }
    }
}

/// Stored object row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredObject {
    /// Object id.
    pub id: String,
    /// Snapshot id.
    pub snapshot_id: String,
    /// Object kind string.
    pub kind: String,
    /// Object name.
    pub name: String,
    /// Fully qualified name.
    pub full_name: String,
    /// Schema name.
    pub schema_name: Option<String>,
    /// Table name.
    pub table_name: Option<String>,
    /// Column name.
    pub column_name: Option<String>,
    /// Metadata JSON.
    pub metadata_json: String,
}

/// Stored edge row.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredEdge {
    /// Edge id.
    pub id: String,
    /// Snapshot id.
    pub snapshot_id: String,
    /// Edge kind string.
    pub kind: String,
    /// Source object id.
    pub from_object_id: String,
    /// Target object id.
    pub to_object_id: String,
    /// Confidence.
    pub confidence: f64,
    /// Metadata JSON.
    pub metadata_json: String,
}

/// Stored SQL artifact row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SqlArtifactRecord {
    /// Artifact id.
    pub id: String,
    /// Snapshot id.
    pub snapshot_id: String,
    /// Source kind.
    pub source_kind: String,
    /// Source path.
    pub source_path: String,
    /// SQL dialect.
    pub dialect: String,
    /// SQL fingerprint.
    pub fingerprint: String,
    /// Normalized SQL.
    pub normalized_sql: String,
    /// Serialized AST summary.
    pub ast_json: String,
    /// Serialized lineage analysis.
    pub analysis_json: String,
}

/// Runs schema migrations and returns whether `FTS5` is enabled.
///
/// # Errors
///
/// Returns an error when required schema statements fail.
#[allow(clippy::too_many_lines)]
pub fn run_migrations(conn: &Connection) -> Result<bool> {
    conn.execute_batch(
        "
        PRAGMA foreign_keys = ON;

        CREATE TABLE IF NOT EXISTS schema_versions (
            version INTEGER PRIMARY KEY,
            applied_at_unix_ms INTEGER NOT NULL,
            description TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS snapshots (
            id TEXT PRIMARY KEY,
            provider TEXT NOT NULL,
            database_name TEXT NOT NULL,
            created_at_unix_ms INTEGER NOT NULL,
            schema_hash TEXT,
            metadata_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS objects (
            id TEXT NOT NULL,
            snapshot_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            full_name TEXT NOT NULL,
            schema_name TEXT,
            table_name TEXT,
            column_name TEXT,
            metadata_json TEXT NOT NULL,
            PRIMARY KEY (snapshot_id, id),
            FOREIGN KEY (snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS edges (
            id TEXT NOT NULL,
            snapshot_id TEXT NOT NULL,
            kind TEXT NOT NULL,
            from_object_id TEXT NOT NULL,
            to_object_id TEXT NOT NULL,
            confidence REAL NOT NULL,
            metadata_json TEXT NOT NULL,
            PRIMARY KEY (snapshot_id, id),
            FOREIGN KEY (snapshot_id) REFERENCES snapshots(id) ON DELETE CASCADE
        );

        CREATE TABLE IF NOT EXISTS table_profiles (
            object_id TEXT NOT NULL,
            snapshot_id TEXT NOT NULL,
            row_estimate INTEGER,
            row_count_kind TEXT,
            size_bytes INTEGER,
            profile_json TEXT NOT NULL,
            PRIMARY KEY (snapshot_id, object_id)
        );

        CREATE TABLE IF NOT EXISTS column_profiles (
            object_id TEXT NOT NULL,
            snapshot_id TEXT NOT NULL,
            data_type_family TEXT,
            null_fraction REAL,
            distinct_estimate REAL,
            pii_score REAL,
            profile_json TEXT NOT NULL,
            PRIMARY KEY (snapshot_id, object_id)
        );

        CREATE TABLE IF NOT EXISTS sql_artifacts (
            id TEXT PRIMARY KEY,
            snapshot_id TEXT NOT NULL,
            source_kind TEXT NOT NULL,
            source_path TEXT NOT NULL,
            dialect TEXT NOT NULL,
            fingerprint TEXT NOT NULL,
            normalized_sql TEXT NOT NULL,
            ast_json TEXT NOT NULL,
            analysis_json TEXT NOT NULL
        );

        CREATE TABLE IF NOT EXISTS project_metadata (
            key TEXT PRIMARY KEY,
            value TEXT NOT NULL,
            updated_at_unix_ms INTEGER NOT NULL
        );

        CREATE INDEX IF NOT EXISTS idx_objects_snapshot_kind_full_name ON objects(snapshot_id, kind, full_name);
        CREATE INDEX IF NOT EXISTS idx_objects_snapshot_full_name ON objects(snapshot_id, full_name);
        CREATE INDEX IF NOT EXISTS idx_edges_snapshot_kind_from ON edges(snapshot_id, kind, from_object_id);
        CREATE INDEX IF NOT EXISTS idx_edges_snapshot_kind_to ON edges(snapshot_id, kind, to_object_id);
        CREATE INDEX IF NOT EXISTS idx_table_profiles_snapshot ON table_profiles(snapshot_id);
        CREATE INDEX IF NOT EXISTS idx_column_profiles_snapshot ON column_profiles(snapshot_id);
        CREATE UNIQUE INDEX IF NOT EXISTS idx_sql_artifacts_snapshot_fingerprint ON sql_artifacts(snapshot_id, fingerprint);
        CREATE INDEX IF NOT EXISTS idx_sql_artifacts_snapshot_source ON sql_artifacts(snapshot_id, source_path);
        ",
    )
    .map_err(sql_error)?;

    let fts_enabled = match conn.execute_batch(
        "
        CREATE VIRTUAL TABLE IF NOT EXISTS object_fts USING fts5(
            snapshot_id UNINDEXED,
            object_id UNINDEXED,
            name,
            full_name,
            content
        );
        ",
    ) {
        Ok(()) => true,
        Err(_) => false,
    };

    conn.execute(
        "INSERT OR IGNORE INTO schema_versions(version, applied_at_unix_ms, description)
         VALUES (?1, ?2, ?3)",
        params![CURRENT_SCHEMA_VERSION, 0_i64, "initial storage schema"],
    )
    .map_err(sql_error)?;
    conn.execute(
        "INSERT OR REPLACE INTO project_metadata(key, value, updated_at_unix_ms)
         VALUES ('fts_enabled', ?1, 0)",
        params![if fts_enabled { "true" } else { "false" }],
    )
    .map_err(sql_error)?;

    Ok(fts_enabled)
}

fn schema_version(conn: &Connection) -> Result<i64> {
    conn.query_row(
        "SELECT COALESCE(MAX(version), 0) FROM schema_versions",
        [],
        |row| row.get(0),
    )
    .map_err(sql_error)
}

fn delete_snapshot_index(tx: &Transaction<'_>, snapshot_id: &str) -> Result<()> {
    tx.execute(
        "DELETE FROM object_fts WHERE snapshot_id = ?1",
        params![snapshot_id],
    )
    .ok();
    for table in [
        "column_profiles",
        "table_profiles",
        "sql_artifacts",
        "edges",
        "objects",
    ] {
        tx.execute(
            &format!("DELETE FROM {table} WHERE snapshot_id = ?1"),
            params![snapshot_id],
        )
        .map_err(sql_error)?;
    }
    tx.execute("DELETE FROM snapshots WHERE id = ?1", params![snapshot_id])
        .map_err(sql_error)?;
    Ok(())
}

fn insert_snapshot_conn(conn: &Connection, snapshot: &DbSnapshot) -> Result<()> {
    conn.execute(
        "INSERT OR REPLACE INTO snapshots(id, provider, database_name, created_at_unix_ms, schema_hash, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot.id,
            snapshot.provider,
            snapshot.database_name,
            i64::try_from(snapshot.created_at_unix_ms).unwrap_or(i64::MAX),
            snapshot.schema_hash,
            serde_json::to_string(&snapshot.metadata).map_err(json_error)?
        ],
    )
    .map(|_| ())
    .map_err(sql_error)
}

fn insert_snapshot_tx(tx: &Transaction<'_>, snapshot: &DbSnapshot) -> Result<()> {
    tx.execute(
        "INSERT OR REPLACE INTO snapshots(id, provider, database_name, created_at_unix_ms, schema_hash, metadata_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        params![
            snapshot.id,
            snapshot.provider,
            snapshot.database_name,
            i64::try_from(snapshot.created_at_unix_ms).unwrap_or(i64::MAX),
            snapshot.schema_hash,
            serde_json::to_string(&snapshot.metadata).map_err(json_error)?
        ],
    )
    .map(|_| ())
    .map_err(sql_error)
}

fn insert_objects_tx(
    tx: &Transaction<'_>,
    snapshot_id: &str,
    objects: &[DbObject],
    fts_enabled: bool,
) -> Result<()> {
    let mut stmt = tx
        .prepare(
            "INSERT INTO objects(id, snapshot_id, kind, name, full_name, schema_name, table_name, column_name, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        )
        .map_err(sql_error)?;
    for object in objects {
        let metadata_json = object_metadata_json(object)?;
        stmt.execute(params![
            object.id,
            snapshot_id,
            object.kind.as_str(),
            object.name,
            object.full_name,
            object.schema_name,
            object.table_name,
            object.column_name,
            metadata_json
        ])
        .map_err(sql_error)?;
    }
    drop(stmt);

    if fts_enabled {
        let mut fts = tx
            .prepare(
                "INSERT INTO object_fts(snapshot_id, object_id, name, full_name, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .map_err(sql_error)?;
        for object in objects {
            fts.execute(params![
                snapshot_id,
                object.id,
                object.name,
                object.full_name,
                fts_content(object)?
            ])
            .map_err(sql_error)?;
        }
    }
    Ok(())
}

fn insert_edges_tx(tx: &Transaction<'_>, snapshot_id: &str, edges: &[DbEdge]) -> Result<()> {
    let mut stmt = tx
        .prepare(
            "INSERT INTO edges(id, snapshot_id, kind, from_object_id, to_object_id, confidence, metadata_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(sql_error)?;
    for edge in edges {
        let metadata_json = serde_json::to_string(&edge.metadata).map_err(json_error)?;
        stmt.execute(params![
            edge.id,
            snapshot_id,
            edge.kind.as_str(),
            edge.from_object_id,
            edge.to_object_id,
            edge.confidence,
            metadata_json
        ])
        .map_err(sql_error)?;
    }
    Ok(())
}

fn insert_table_profiles_tx(
    tx: &Transaction<'_>,
    snapshot_id: &str,
    profiles: &[TableProfile],
) -> Result<()> {
    let mut stmt = tx
        .prepare(
            "INSERT INTO table_profiles(object_id, snapshot_id, row_estimate, row_count_kind, size_bytes, profile_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .map_err(sql_error)?;
    for profile in profiles {
        let profile_json = serde_json::to_string(&profile.profile).map_err(json_error)?;
        stmt.execute(params![
            profile.object_id,
            snapshot_id,
            profile.row_estimate,
            profile.row_count_kind,
            profile.size_bytes,
            profile_json
        ])
        .map_err(sql_error)?;
    }
    Ok(())
}

fn insert_column_profiles_tx(
    tx: &Transaction<'_>,
    snapshot_id: &str,
    profiles: &[ColumnProfile],
) -> Result<()> {
    let mut stmt = tx
        .prepare(
            "INSERT INTO column_profiles(object_id, snapshot_id, data_type_family, null_fraction, distinct_estimate, pii_score, profile_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        )
        .map_err(sql_error)?;
    for profile in profiles {
        let profile_json = serde_json::to_string(&profile.profile).map_err(json_error)?;
        stmt.execute(params![
            profile.object_id,
            snapshot_id,
            profile.data_type_family,
            profile.null_fraction,
            profile.distinct_estimate,
            profile.pii_score,
            profile_json
        ])
        .map_err(sql_error)?;
    }
    Ok(())
}

fn object_metadata_json(object: &DbObject) -> Result<String> {
    serde_json::json!({
        "table": object.table,
        "column": object.column,
        "index": object.index,
        "constraint": object.constraint,
        "metadata": object.metadata,
    })
    .to_string()
    .pipe(Ok)
}

fn fts_content(object: &DbObject) -> Result<String> {
    let metadata_json = object_metadata_json(object)?;
    Ok(format!(
        "{} {} {}",
        object.name, object.full_name, metadata_json
    ))
}

fn row_to_object(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredObject> {
    Ok(StoredObject {
        id: row.get(0)?,
        snapshot_id: row.get(1)?,
        kind: row.get(2)?,
        name: row.get(3)?,
        full_name: row.get(4)?,
        schema_name: row.get(5)?,
        table_name: row.get(6)?,
        column_name: row.get(7)?,
        metadata_json: row.get(8)?,
    })
}

fn row_to_edge(row: &rusqlite::Row<'_>) -> rusqlite::Result<StoredEdge> {
    Ok(StoredEdge {
        id: row.get(0)?,
        snapshot_id: row.get(1)?,
        kind: row.get(2)?,
        from_object_id: row.get(3)?,
        to_object_id: row.get(4)?,
        confidence: row.get(5)?,
        metadata_json: row.get(6)?,
    })
}

fn row_to_sql_artifact(row: &rusqlite::Row<'_>) -> rusqlite::Result<SqlArtifactRecord> {
    Ok(SqlArtifactRecord {
        id: row.get(0)?,
        snapshot_id: row.get(1)?,
        source_kind: row.get(2)?,
        source_path: row.get(3)?,
        dialect: row.get(4)?,
        fingerprint: row.get(5)?,
        normalized_sql: row.get(6)?,
        ast_json: row.get(7)?,
        analysis_json: row.get(8)?,
    })
}

fn count_by_snapshot(conn: &Connection, table: &str, snapshot_id: &str) -> Result<i64> {
    conn.query_row(
        &format!("SELECT COUNT(*) FROM {table} WHERE snapshot_id = ?1"),
        params![snapshot_id],
        |row| row.get(0),
    )
    .map_err(sql_error)
}

#[allow(clippy::needless_pass_by_value)]
fn sql_error(source: rusqlite::Error) -> DbGraphError {
    DbGraphError::Internal {
        message: format!("SQLite storage error: {source}"),
    }
}

#[allow(clippy::needless_pass_by_value)]
fn json_error(source: serde_json::Error) -> DbGraphError {
    DbGraphError::Internal {
        message: format!("JSON serialization error: {source}"),
    }
}

trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}

impl<T> Pipe for T {}

#[cfg(test)]
mod tests {
    use super::*;
    use dbgraph_core::model::{ColumnMetadata, DbEdgeKind, Metadata};

    #[test]
    fn migrations_are_idempotent() {
        let repo = GraphRepository::open_in_memory().expect("repo should open");
        run_migrations(&repo.conn).expect("second migration should pass");

        assert_eq!(repo.schema_version().unwrap(), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn rebuild_snapshot_inserts_and_rebuilds_counts() {
        let mut repo = GraphRepository::open_in_memory().expect("repo should open");
        let snapshot = sample_snapshot(10);

        repo.rebuild_snapshot(&snapshot)
            .expect("snapshot should rebuild");
        repo.rebuild_snapshot(&snapshot)
            .expect("snapshot should rebuild again");

        assert_eq!(repo.object_count(&snapshot.id).unwrap(), 10);
        assert_eq!(repo.edge_count(&snapshot.id).unwrap(), 9);
    }

    #[test]
    fn query_results_are_stably_sorted() {
        let mut repo = GraphRepository::open_in_memory().expect("repo should open");
        let mut snapshot = sample_snapshot(3);
        snapshot.objects.reverse();
        repo.rebuild_snapshot(&snapshot)
            .expect("snapshot should rebuild");

        let objects = repo.objects_by_snapshot(&snapshot.id).unwrap();
        let names = objects
            .iter()
            .map(|object| object.full_name.as_str())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec![
                "public.items.item_0_id",
                "public.items.item_1_id",
                "public.items.item_2_id"
            ]
        );
    }

    #[test]
    fn fts_or_fallback_search_finds_comments() {
        let mut repo = GraphRepository::open_in_memory().expect("repo should open");
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        let mut object = DbObject::new("table:orders", DbObjectKind::Table, "public.orders");
        object.metadata.insert(
            "comment".to_owned(),
            serde_json::Value::String("customer checkout order records".to_owned()),
        );
        snapshot.objects.push(object);
        repo.rebuild_snapshot(&snapshot)
            .expect("snapshot should rebuild");

        let results = repo.search_objects("checkout").expect("search should work");

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].full_name, "public.orders");
    }

    #[test]
    fn sql_artifacts_are_deduped_by_snapshot_and_fingerprint() {
        let mut repo = GraphRepository::open_in_memory().expect("repo should open");
        let snapshot = sample_snapshot(1);
        repo.rebuild_snapshot(&snapshot)
            .expect("snapshot should rebuild");
        let artifact = SqlArtifactRecord {
            id: "sql:one".to_owned(),
            snapshot_id: snapshot.id.clone(),
            source_kind: "file".to_owned(),
            source_path: "sql/users.sql".to_owned(),
            dialect: "postgres".to_owned(),
            fingerprint: "abc123".to_owned(),
            normalized_sql: "SELECT * FROM users".to_owned(),
            ast_json: "{}".to_owned(),
            analysis_json: "{}".to_owned(),
        };

        repo.insert_sql_artifacts(std::slice::from_ref(&artifact))
            .expect("first insert should pass");
        let mut updated = artifact;
        updated.id = "sql:two".to_owned();
        updated.normalized_sql = "SELECT * FROM public.users".to_owned();
        repo.insert_sql_artifacts(&[updated])
            .expect("second insert should replace fingerprint");

        let stored = repo
            .sql_artifacts_by_snapshot(&snapshot.id)
            .expect("artifacts should load");

        assert_eq!(stored.len(), 1);
        assert_eq!(stored[0].id, "sql:two");
        assert_eq!(stored[0].normalized_sql, "SELECT * FROM public.users");
    }

    #[test]
    fn bulk_insert_10k_objects_is_supported() {
        let mut repo = GraphRepository::open_in_memory().expect("repo should open");
        let snapshot = sample_snapshot(10_000);

        repo.rebuild_snapshot(&snapshot)
            .expect("bulk rebuild should pass");

        assert_eq!(repo.object_count(&snapshot.id).unwrap(), 10_000);
    }

    #[test]
    fn failed_rebuild_rolls_back_previous_snapshot_index() {
        let mut repo = GraphRepository::open_in_memory().expect("repo should open");
        let valid = sample_snapshot(2);
        repo.rebuild_snapshot(&valid)
            .expect("valid rebuild should pass");

        let mut invalid = sample_snapshot(2);
        invalid.objects[1].id = invalid.objects[0].id.clone();

        let error = repo
            .rebuild_snapshot(&invalid)
            .expect_err("duplicate object ids should fail");

        assert!(error.to_string().contains("SQLite storage error"));
        assert_eq!(repo.object_count(&valid.id).unwrap(), 2);
        assert_eq!(repo.edge_count(&valid.id).unwrap(), 1);
    }

    fn sample_snapshot(object_count: usize) -> DbSnapshot {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        for idx in 0..object_count {
            let mut object = DbObject::new(
                format!("column:{idx}"),
                DbObjectKind::Column,
                format!("public.items.item_{idx}_id"),
            );
            object.schema_name = Some("public".to_owned());
            object.table_name = Some("items".to_owned());
            object.column_name = Some(format!("item_{idx}_id"));
            object.column = Some(ColumnMetadata {
                data_type: Some("bigint".to_owned()),
                data_type_family: Some("integer".to_owned()),
                nullable: Some(false),
                default: None,
                comment: Some("indexed item reference".to_owned()),
            });
            object.metadata = Metadata::new();
            snapshot.objects.push(object);
            if idx > 0 {
                snapshot.edges.push(DbEdge::explicit(
                    format!("edge:{idx}"),
                    DbEdgeKind::References,
                    format!("column:{idx}"),
                    format!("column:{}", idx - 1),
                ));
            }
        }
        snapshot
    }
}

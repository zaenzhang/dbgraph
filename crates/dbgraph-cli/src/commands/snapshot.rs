//! Snapshot and sync CLI command implementations.

use std::env;
use std::path::{Path, PathBuf};

use dbgraph_core::config::{DatabaseConfig, DbGraphConfig};
use dbgraph_core::model::DbSnapshot;
use dbgraph_core::profiling::{apply_profiling_policy, ProfilingMode, ProfilingOptions};
use dbgraph_core::project::ProjectContext;
use dbgraph_core::sampling::{SamplingOptions, SamplingStrategy};
use dbgraph_core::security::{apply_pii_profiles, MaskingStrategy, PiiDetector, PiiRuleConfig};
use dbgraph_core::snapshot::{now_unix_ms, SnapshotStore};
use dbgraph_core::sync::{plan_incremental_sync, SyncPlan};
use dbgraph_core::{DbGraphError, Result};
use dbgraph_graph::rebuild_index;
use dbgraph_provider::{ProviderConnectionConfig, ProviderRegistry};
use dbgraph_sql::{
    analyze_sql, resolve_sql_edge_targets, scan_sql_files, sql_artifact_to_graph, ScanOptions,
    SqlDialect, SqlParser,
};
use dbgraph_storage::{GraphRepository, SqlArtifactRecord as StoredSqlArtifactRecord};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SnapshotSummary {
    pub(crate) project_root: PathBuf,
    pub(crate) snapshot_path: PathBuf,
    pub(crate) graph_db_path: PathBuf,
    pub(crate) provider: String,
    pub(crate) database_name: String,
    pub(crate) object_count: usize,
    pub(crate) table_count: usize,
    pub(crate) column_count: usize,
    pub(crate) edge_count: usize,
    pub(crate) table_profile_count: usize,
    pub(crate) column_profile_count: usize,
    pub(crate) sql_artifact_count: usize,
    pub(crate) profiling_mode: String,
    pub(crate) schema_hash: Option<String>,
}

pub(crate) fn run_snapshot(start: impl AsRef<Path>) -> Result<SnapshotSummary> {
    run_snapshot_with_options(start, SnapshotCliOptions::default())
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SnapshotCliOptions {
    pub(crate) profile: Option<ProfilingMode>,
    pub(crate) max_rows_per_table: Option<u32>,
    pub(crate) store_raw_samples: bool,
}

pub(crate) fn run_snapshot_with_options(
    start: impl AsRef<Path>,
    cli_options: SnapshotCliOptions,
) -> Result<SnapshotSummary> {
    let start = start.as_ref();
    let context = ProjectContext::discover_from(start)?
        .unwrap_or_else(|| ProjectContext::from_project_root(start));
    let config = load_config_with_snapshot_overrides(&context, cli_options)?;
    let (snapshot, sql_artifacts, profiling_options) = capture_snapshot(&context, &config)?;
    write_snapshot_and_index(
        &context,
        &config,
        &snapshot,
        &sql_artifacts,
        &profiling_options,
    )
}

fn load_config_with_snapshot_overrides(
    context: &ProjectContext,
    cli_options: SnapshotCliOptions,
) -> Result<DbGraphConfig> {
    let mut config = DbGraphConfig::load(context)?;
    if let Some(profile) = cli_options.profile {
        config.snapshot.profiling_mode = profile;
        config.snapshot.sample_rows = profile == ProfilingMode::Sample;
    }
    if let Some(max_rows_per_table) = cli_options.max_rows_per_table {
        config.snapshot.max_rows_per_table = max_rows_per_table;
    }
    if cli_options.store_raw_samples {
        config.security.store_raw_samples = true;
    }
    config.validate()?;
    config.database.provider_kind()?;
    Ok(config)
}

fn effective_profiling_options(config: &DbGraphConfig) -> ProfilingOptions {
    ProfilingOptions {
        mode: config.snapshot.profiling_mode,
        max_rows_per_table: config.snapshot.max_rows_per_table,
        mask_pii: config.security.mask_pii,
        store_raw_samples: config.security.store_raw_samples,
    }
}

fn capture_snapshot(
    context: &ProjectContext,
    config: &DbGraphConfig,
) -> Result<(DbSnapshot, Vec<StoredSqlArtifactRecord>, ProfilingOptions)> {
    let profiling_options = effective_profiling_options(config);

    let registry = ProviderRegistry;
    let provider = registry.get(&config.database.provider).ok_or_else(|| {
        DbGraphError::invalid_config(format!(
            "provider `{}` is not registered",
            config.database.provider
        ))
    })?;
    let connection_url = resolve_connection_url(&config.database)?;
    let connection = ProviderConnectionConfig::from_url(connection_url);
    let sampling_options = SamplingOptions {
        max_rows_per_table: config.snapshot.max_rows_per_table,
        strategy: SamplingStrategy::Limit,
        statement_timeout_ms: Some(connection.statement_timeout_ms),
        store_raw_samples: config.security.store_raw_samples,
        masking_strategy: MaskingStrategy::Redact,
    };
    let mut snapshot =
        provider.snapshot_with_data_access(&connection, &config.data_access, &sampling_options)?;
    let timestamp = now_unix_ms()?;
    snapshot.created_at_unix_ms = timestamp;
    snapshot.id = format!(
        "{}:{}:{timestamp}",
        snapshot.provider, snapshot.database_name
    );

    let sql_artifacts = enrich_snapshot_with_sql(&mut snapshot, context)?;
    snapshot = apply_profiling_policy(snapshot, &profiling_options);
    if config.security.mask_pii {
        let detector = PiiDetector::new(&PiiRuleConfig {
            custom_sensitive_terms: config.security.custom_sensitive_terms.clone(),
        });
        snapshot = apply_pii_profiles(snapshot, &detector);
    }
    Ok((snapshot, sql_artifacts, profiling_options))
}

fn write_snapshot_and_index(
    context: &ProjectContext,
    config: &DbGraphConfig,
    snapshot: &DbSnapshot,
    sql_artifacts: &[StoredSqlArtifactRecord],
    profiling_options: &ProfilingOptions,
) -> Result<SnapshotSummary> {
    let snapshot_path =
        SnapshotStore::new(context).write_snapshot(snapshot, config.snapshot.pretty_json)?;
    let stored_snapshot = SnapshotStore::new(context).read_snapshot(&snapshot_path)?;
    let mut repository = GraphRepository::open(context.graph_db_path())?;
    let index_summary = rebuild_index(&mut repository, &stored_snapshot)?;
    repository.insert_sql_artifacts(sql_artifacts)?;

    Ok(SnapshotSummary {
        project_root: context.project_root().to_path_buf(),
        snapshot_path,
        graph_db_path: context.graph_db_path(),
        provider: stored_snapshot.provider.clone(),
        database_name: stored_snapshot.database_name.clone(),
        object_count: index_summary.object_count,
        table_count: stored_snapshot
            .objects
            .iter()
            .filter(|object| object.kind.as_str() == "table")
            .count(),
        column_count: stored_snapshot
            .objects
            .iter()
            .filter(|object| object.kind.as_str() == "column")
            .count(),
        edge_count: index_summary.edge_count,
        table_profile_count: index_summary.table_profile_count,
        column_profile_count: index_summary.column_profile_count,
        sql_artifact_count: sql_artifacts.len(),
        profiling_mode: profiling_options.mode.to_string(),
        schema_hash: stored_snapshot.schema_hash,
    })
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SyncSummary {
    pub(crate) project_root: PathBuf,
    pub(crate) plan: SyncPlan,
    pub(crate) snapshot: Option<SnapshotSummary>,
}

pub(crate) fn sync_project(start: impl AsRef<Path>) -> Result<SyncSummary> {
    let start = start.as_ref();
    let context = ProjectContext::discover_from(start)?
        .unwrap_or_else(|| ProjectContext::from_project_root(start));
    let previous = SnapshotStore::new(&context).read_latest()?;
    let config = load_config_with_snapshot_overrides(&context, SnapshotCliOptions::default())?;
    let (snapshot, sql_artifacts, profiling_options) = capture_snapshot(&context, &config)?;
    let plan = plan_incremental_sync(previous.as_ref(), &snapshot)?;
    let snapshot = if plan.can_skip_rebuild() {
        None
    } else {
        Some(write_snapshot_and_index(
            &context,
            &config,
            &snapshot,
            &sql_artifacts,
            &profiling_options,
        )?)
    };

    Ok(SyncSummary {
        project_root: context.project_root().to_path_buf(),
        snapshot,
        plan,
    })
}

fn enrich_snapshot_with_sql(
    snapshot: &mut dbgraph_core::model::DbSnapshot,
    context: &ProjectContext,
) -> Result<Vec<StoredSqlArtifactRecord>> {
    let sources = scan_sql_files(context.project_root(), &ScanOptions::default())?;
    let dialect = dialect_for_provider(&snapshot.provider);
    let mut artifacts = Vec::new();
    for source in sources {
        let parser = SqlParser::new(dialect);
        let parsed = parser.parse(&source.raw_sql)?;
        let analysis = analyze_sql(&source.raw_sql, dialect)?;
        let source_path = source.source_path.to_string_lossy().replace('\\', "/");
        let mut graph = sql_artifact_to_graph(&snapshot.id, &source_path, &parsed, &analysis)?;
        resolve_sql_edge_targets(snapshot, &mut graph.edges);
        snapshot.objects.push(graph.object);
        snapshot.edges.extend(graph.edges);
        artifacts.push(StoredSqlArtifactRecord {
            id: graph.artifact.id,
            snapshot_id: graph.artifact.snapshot_id,
            source_kind: graph.artifact.source_kind,
            source_path: graph.artifact.source_path,
            dialect: graph.artifact.dialect,
            fingerprint: graph.artifact.fingerprint,
            normalized_sql: graph.artifact.normalized_sql,
            ast_json: graph.artifact.ast_json,
            analysis_json: graph.artifact.analysis_json,
        });
    }
    Ok(artifacts)
}

fn dialect_for_provider(provider: &str) -> SqlDialect {
    match provider {
        "postgres" => SqlDialect::Postgres,
        "mysql" => SqlDialect::MySql,
        _ => SqlDialect::Generic,
    }
}

pub(crate) fn resolve_connection_url(config: &DatabaseConfig) -> Result<String> {
    if let Some(env_name) = config.connection_env.as_deref() {
        if let Ok(value) = env::var(env_name) {
            if !value.trim().is_empty() {
                return Ok(value);
            }
        }
    }
    config.connection_string.clone().ok_or_else(|| {
        DbGraphError::invalid_config(
            "database connection string is missing; set DATABASE_URL or database.connectionString",
        )
    })
}

pub(crate) fn print_snapshot_summary(summary: &SnapshotSummary) {
    println!("DbGraph snapshot complete");
    println!("Project: {}", summary.project_root.display());
    println!("Provider: {}", summary.provider);
    println!("Database: {}", summary.database_name);
    println!("Snapshot: {}", summary.snapshot_path.display());
    println!("Graph index: {}", summary.graph_db_path.display());
    println!("Objects: {}", summary.object_count);
    println!("Tables: {}", summary.table_count);
    println!("Columns: {}", summary.column_count);
    println!("Edges: {}", summary.edge_count);
    println!("Table profiles: {}", summary.table_profile_count);
    println!("Column profiles: {}", summary.column_profile_count);
    println!("SQL artifacts: {}", summary.sql_artifact_count);
    println!("Profiling mode: {}", summary.profiling_mode);
    if let Some(hash) = &summary.schema_hash {
        println!("Schema hash: {hash}");
    }
}

pub(crate) fn print_sync_summary(summary: &SyncSummary) {
    println!("DbGraph sync complete");
    println!("Project: {}", summary.project_root.display());
    match &summary.plan {
        SyncPlan::Unchanged { schema_hash } => {
            println!("Schema unchanged: {schema_hash}");
            println!("Skipped snapshot write and graph index rebuild");
        }
        SyncPlan::Changed {
            previous_hash,
            next_hash,
        } => {
            println!("Schema changed");
            println!(
                "Previous hash: {}",
                previous_hash.as_deref().unwrap_or("<none>")
            );
            println!("Next hash: {next_hash}");
        }
    }
}

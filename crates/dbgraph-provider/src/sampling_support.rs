//! Shared helpers for provider row-sample summaries.

use dbgraph_core::model::{ColumnProfile, DbSnapshot};
use dbgraph_core::sampling::ColumnSampleSummary;
use dbgraph_core::{DbGraphError, Result};

pub(crate) fn upsert_sample_summary(
    snapshot: &mut DbSnapshot,
    object_id: &str,
    summary: &ColumnSampleSummary,
) -> Result<()> {
    let value = serde_json::to_value(summary).map_err(|source| DbGraphError::Internal {
        message: format!("failed to serialize sample summary: {source}"),
    })?;
    if let Some(profile) = snapshot
        .column_profiles
        .iter_mut()
        .find(|profile| profile.object_id == object_id)
    {
        profile.profile.insert("sampleSummary".to_owned(), value);
        return Ok(());
    }
    let data_type_family = snapshot
        .objects
        .iter()
        .find(|object| object.id == object_id)
        .and_then(|object| object.column.as_ref())
        .and_then(|column| column.data_type_family.clone());
    snapshot.column_profiles.push(ColumnProfile {
        object_id: object_id.to_owned(),
        data_type_family,
        null_fraction: None,
        distinct_estimate: None,
        pii_score: None,
        profile: [("sampleSummary".to_owned(), value)].into_iter().collect(),
    });
    Ok(())
}

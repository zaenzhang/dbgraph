//! Project-owned semantic metadata for database graph objects.

use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::model::{DbSnapshot, Metadata};
use crate::{DbGraphError, Result};

/// Semantic metadata file format.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticMetadataConfig {
    /// Config schema version.
    pub version: u32,
    /// Object-specific semantic metadata.
    #[serde(default)]
    pub objects: Vec<SemanticObject>,
}

impl Default for SemanticMetadataConfig {
    fn default() -> Self {
        Self {
            version: 1,
            objects: Vec::new(),
        }
    }
}

impl SemanticMetadataConfig {
    /// Loads semantic metadata from a path if it exists.
    ///
    /// # Errors
    ///
    /// Returns an I/O or config error when the file exists but cannot be read
    /// or parsed.
    pub fn load_optional(path: impl AsRef<Path>) -> Result<Option<Self>> {
        let path = path.as_ref();
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(path).map_err(|source| DbGraphError::io(path, source))?;
        let config = serde_json::from_str::<Self>(&content).map_err(|source| {
            DbGraphError::invalid_config(format!(
                "failed to parse semantic metadata {}: {source}",
                path.display()
            ))
        })?;
        config.validate()?;
        Ok(Some(config))
    }

    /// Applies matching semantic metadata to objects in a snapshot.
    ///
    /// # Errors
    ///
    /// Returns an invalid config error when semantic metadata is invalid.
    pub fn apply_to_snapshot(&self, snapshot: &mut DbSnapshot) -> Result<()> {
        self.validate()?;
        for rule in &self.objects {
            if let Some(object) = snapshot.objects.iter_mut().find(|object| {
                object.full_name == rule.object
                    || object.id == rule.object
                    || object.name == rule.object
            }) {
                object.metadata.insert(
                    "semantic".to_owned(),
                    serde_json::to_value(&rule.metadata).map_err(|source| {
                        DbGraphError::Internal {
                            message: format!("failed to serialize semantic metadata: {source}"),
                        }
                    })?,
                );
            }
        }
        if !self.objects.is_empty() {
            snapshot.metadata.insert(
                "semanticMetadata".to_owned(),
                serde_json::json!({
                    "version": self.version,
                    "objectCount": self.objects.len()
                }),
            );
        }
        Ok(())
    }

    fn validate(&self) -> Result<()> {
        if self.version == 0 {
            return Err(DbGraphError::invalid_config(
                "semantics.version must be greater than zero",
            ));
        }
        for object in &self.objects {
            if object.object.trim().is_empty() {
                return Err(DbGraphError::invalid_config(
                    "semantics.objects[].object must not be empty",
                ));
            }
        }
        Ok(())
    }
}

/// Semantic metadata for one graph object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SemanticObject {
    /// Object id, name, or fully-qualified name.
    pub object: String,
    /// Metadata attached to the object.
    #[serde(flatten)]
    pub metadata: SemanticMetadata,
}

/// Business semantic metadata attached to an object.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SemanticMetadata {
    /// Human-readable business meaning.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Owning team or person.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// Business domain.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Sensitivity label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sensitivity: Option<String>,
    /// Allowed values for enum-like fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
    /// Optional replacement object when deprecated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replacement: Option<String>,
    /// Whether the object is deprecated.
    #[serde(default)]
    pub deprecated: bool,
    /// Whether the object is certified as preferred.
    #[serde(default)]
    pub certified: bool,
    /// Additional project-specific semantic metadata.
    #[serde(default, skip_serializing_if = "Metadata::is_empty")]
    pub metadata: Metadata,
}

/// Returns semantic metadata from an object metadata map.
#[must_use]
pub fn semantic_metadata(metadata: &Metadata) -> Option<&Value> {
    metadata.get("semantic")
}

/// Returns whether object metadata marks the object deprecated.
#[must_use]
pub fn is_deprecated(metadata: &Metadata) -> bool {
    semantic_metadata(metadata)
        .and_then(|semantic| semantic.get("deprecated"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// Returns the semantic description if present.
#[must_use]
pub fn semantic_description(metadata: &Metadata) -> Option<&str> {
    semantic_metadata(metadata)
        .and_then(|semantic| semantic.get("description"))
        .and_then(Value::as_str)
}

#[cfg(test)]
mod tests {
    use crate::model::{DbObject, DbObjectKind, DbSnapshot};
    use crate::semantics::{SemanticMetadata, SemanticMetadataConfig, SemanticObject};

    #[test]
    fn semantic_metadata_merges_matching_objects_without_replacing_provider_metadata() {
        let mut snapshot = DbSnapshot::new("s1", "postgres", "app", 1);
        let mut object = DbObject::new(
            "column:orders.status",
            DbObjectKind::Column,
            "public.orders.status",
        );
        object
            .metadata
            .insert("comment".to_owned(), serde_json::json!("provider comment"));
        snapshot.objects.push(object);

        let config = SemanticMetadataConfig {
            version: 1,
            objects: vec![SemanticObject {
                object: "public.orders.status".to_owned(),
                metadata: SemanticMetadata {
                    description: Some("Order lifecycle state".to_owned()),
                    owner: Some("commerce".to_owned()),
                    allowed_values: vec![
                        "pending".to_owned(),
                        "paid".to_owned(),
                        "shipped".to_owned(),
                    ],
                    deprecated: false,
                    certified: true,
                    ..SemanticMetadata::default()
                },
            }],
        };

        config
            .apply_to_snapshot(&mut snapshot)
            .expect("semantics should merge");

        let metadata = &snapshot.objects[0].metadata;
        assert_eq!(
            metadata.get("comment").and_then(serde_json::Value::as_str),
            Some("provider comment")
        );
        assert_eq!(
            metadata
                .get("semantic")
                .and_then(|value| value.get("description"))
                .and_then(serde_json::Value::as_str),
            Some("Order lifecycle state")
        );
        assert_eq!(
            metadata
                .get("semantic")
                .and_then(|value| value.get("allowedValues"))
                .and_then(serde_json::Value::as_array)
                .map(Vec::len),
            Some(3)
        );
    }
}

//! Project path discovery and local `.dbgraph` layout helpers.

use std::fs;
use std::path::{Component, Path, PathBuf};

use crate::{DbGraphError, Result};

/// Local directory name used for project-scoped `DbGraph` state.
pub const DBGRAPH_DIR_NAME: &str = ".dbgraph";

/// Default configuration file name inside `.dbgraph`.
pub const CONFIG_FILE_NAME: &str = "dbgraph.config.json";

/// Local `SQLite` graph index file name inside `.dbgraph`.
pub const GRAPH_DB_FILE_NAME: &str = "dbgraph.db";

/// Snapshot directory name inside `.dbgraph`.
pub const SNAPSHOTS_DIR_NAME: &str = "snapshots";

/// Agent instruction directory name inside `.dbgraph`.
pub const INSTRUCTIONS_DIR_NAME: &str = "instructions";

/// Project semantic metadata file name inside `.dbgraph`.
pub const SEMANTICS_FILE_NAME: &str = "semantics.json";

/// Project root and derived local `DbGraph` paths.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectContext {
    project_root: PathBuf,
    dbgraph_dir: PathBuf,
}

impl ProjectContext {
    /// Creates a context from a known project root.
    #[must_use]
    pub fn from_project_root(project_root: impl Into<PathBuf>) -> Self {
        let project_root = normalize_path(&project_root.into());
        let dbgraph_dir = project_root.join(DBGRAPH_DIR_NAME);

        Self {
            project_root,
            dbgraph_dir,
        }
    }

    /// Creates a context from an existing `.dbgraph` directory path.
    #[must_use]
    pub fn from_dbgraph_dir(dbgraph_dir: impl Into<PathBuf>) -> Self {
        let dbgraph_dir = normalize_path(&dbgraph_dir.into());
        let project_root = dbgraph_dir
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);

        Self {
            project_root,
            dbgraph_dir,
        }
    }

    /// Finds the nearest ancestor project containing a `.dbgraph` directory.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the start path cannot be resolved.
    pub fn discover_from(start: impl AsRef<Path>) -> Result<Option<Self>> {
        let start = start.as_ref();
        let start = fs::canonicalize(start).map_err(|source| DbGraphError::io(start, source))?;
        Ok(find_dbgraph_dir_from(&start).map(Self::from_dbgraph_dir))
    }

    /// Returns the project root path.
    #[must_use]
    pub fn project_root(&self) -> &Path {
        &self.project_root
    }

    /// Returns the `.dbgraph` directory path.
    #[must_use]
    pub fn dbgraph_dir(&self) -> &Path {
        &self.dbgraph_dir
    }

    /// Returns `.dbgraph/dbgraph.config.json`.
    #[must_use]
    pub fn config_path(&self) -> PathBuf {
        self.dbgraph_dir.join(CONFIG_FILE_NAME)
    }

    /// Returns `.dbgraph/dbgraph.db`.
    #[must_use]
    pub fn graph_db_path(&self) -> PathBuf {
        self.dbgraph_dir.join(GRAPH_DB_FILE_NAME)
    }

    /// Returns `.dbgraph/snapshots`.
    #[must_use]
    pub fn snapshots_dir(&self) -> PathBuf {
        self.dbgraph_dir.join(SNAPSHOTS_DIR_NAME)
    }

    /// Returns `.dbgraph/instructions`.
    #[must_use]
    pub fn instructions_dir(&self) -> PathBuf {
        self.dbgraph_dir.join(INSTRUCTIONS_DIR_NAME)
    }

    /// Returns `.dbgraph/semantics.json`.
    #[must_use]
    pub fn semantics_path(&self) -> PathBuf {
        self.dbgraph_dir.join(SEMANTICS_FILE_NAME)
    }
}

/// Finds the nearest `.dbgraph` directory walking upward from `start`.
#[must_use]
pub fn find_dbgraph_dir_from(start: impl AsRef<Path>) -> Option<PathBuf> {
    let mut current = normalize_path(start.as_ref());

    if current
        .file_name()
        .is_some_and(|name| name == DBGRAPH_DIR_NAME)
    {
        if is_dbgraph_dir(&current) {
            return Some(current);
        }
        current.pop();
    }

    loop {
        let candidate = current.join(DBGRAPH_DIR_NAME);
        if is_dbgraph_dir(&candidate) {
            return Some(candidate);
        }

        if !current.pop() {
            return None;
        }
    }
}

/// Returns whether a path is an existing `.dbgraph` directory.
#[must_use]
pub fn is_dbgraph_dir(path: impl AsRef<Path>) -> bool {
    let path = path.as_ref();
    path.file_name()
        .is_some_and(|name| name == DBGRAPH_DIR_NAME)
        && path.is_dir()
}

fn normalize_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if raw.contains('\\') && path.components().count() <= 1 {
        return normalize_backslash_path(&raw);
    }

    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                normalized.pop();
            }
            Component::Prefix(prefix) => normalized.push(prefix.as_os_str()),
            Component::RootDir => normalized.push(component.as_os_str()),
            Component::Normal(part) => normalized.push(part),
        }
    }

    if normalized.as_os_str().is_empty() {
        PathBuf::from(".")
    } else {
        normalized
    }
}

fn normalize_backslash_path(raw: &str) -> PathBuf {
    let path = raw.replace('\\', "/");
    let mut prefix = String::new();
    let mut absolute = false;
    let mut remainder = path.as_str();

    if let Some(without_slashes) = path.strip_prefix("//") {
        let mut segments = without_slashes.splitn(3, '/');
        let server = segments.next().unwrap_or_default();
        let share = segments.next().unwrap_or_default();
        if !server.is_empty() && !share.is_empty() {
            prefix = format!("//{server}/{share}");
            absolute = true;
            remainder = segments.next().unwrap_or_default();
        }
    } else if path.len() >= 2 && path.as_bytes()[1] == b':' {
        path[..2].clone_into(&mut prefix);
        remainder = &path[2..];
        if let Some(stripped) = remainder.strip_prefix('/') {
            absolute = true;
            remainder = stripped;
        }
    } else if let Some(stripped) = path.strip_prefix('/') {
        absolute = true;
        remainder = stripped;
    }

    let mut parts = Vec::new();
    for part in remainder.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|previous| *previous != "..") {
                    parts.pop();
                } else if !absolute {
                    parts.push(part);
                }
            }
            _ => parts.push(part),
        }
    }

    let mut rendered = String::new();
    if !prefix.is_empty() {
        rendered.push_str(&prefix);
        if absolute {
            rendered.push('/');
        }
    } else if absolute {
        rendered.push('/');
    }

    rendered.push_str(&parts.join("/"));

    if rendered.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(rendered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn derived_paths_use_expected_layout() {
        let context = ProjectContext::from_project_root(PathBuf::from("/repo"));

        assert_path_ends_with(context.dbgraph_dir(), &["repo", ".dbgraph"]);
        assert_path_ends_with(
            &context.config_path(),
            &["repo", ".dbgraph", "dbgraph.config.json"],
        );
        assert_path_ends_with(
            &context.graph_db_path(),
            &["repo", ".dbgraph", "dbgraph.db"],
        );
        assert_path_ends_with(&context.snapshots_dir(), &["repo", ".dbgraph", "snapshots"]);
        assert_path_ends_with(
            &context.instructions_dir(),
            &["repo", ".dbgraph", "instructions"],
        );
    }

    #[test]
    fn finds_project_dbgraph_from_nested_directory() {
        let temp = TempProject::new();
        let nested = temp.root.join("app").join("src").join("queries");
        fs::create_dir_all(&nested).expect("nested directory should be created");
        fs::create_dir(temp.root.join(DBGRAPH_DIR_NAME)).expect(".dbgraph should be created");

        let found = ProjectContext::discover_from(&nested)
            .expect("discovery should not fail")
            .expect("project should be discovered");

        let expected_root = fs::canonicalize(&temp.root).expect("root should canonicalize");
        assert_eq!(found.project_root(), expected_root.as_path());
        assert_eq!(
            found.dbgraph_dir(),
            expected_root.join(DBGRAPH_DIR_NAME).as_path()
        );
    }

    #[test]
    fn returns_none_when_no_dbgraph_directory_exists() {
        let temp = TempProject::new();
        let nested = temp.root.join("app");
        fs::create_dir_all(&nested).expect("nested directory should be created");

        let found = ProjectContext::discover_from(&nested).expect("discovery should not fail");

        assert!(found.is_none());
    }

    #[test]
    fn unix_style_paths_are_normalized() {
        let context = ProjectContext::from_project_root(PathBuf::from("/workspace/app/../service"));

        assert_path_ends_with(context.project_root(), &["workspace", "service"]);
        assert_path_ends_with(context.dbgraph_dir(), &["workspace", "service", ".dbgraph"]);
    }

    #[test]
    fn windows_style_paths_preserve_layout_segments() {
        let context =
            ProjectContext::from_project_root(PathBuf::from(r"C:\Users\dev\project\..\dbgraph"));
        let rendered = context.config_path().to_string_lossy().replace('\\', "/");

        assert!(rendered.contains("C:/Users/dev/dbgraph/.dbgraph/dbgraph.config.json"));
    }

    #[test]
    fn windows_unc_paths_preserve_share_prefix() {
        let context =
            ProjectContext::from_project_root(PathBuf::from(r"\\server\share\project\..\dbgraph"));
        let rendered = context.project_root().to_string_lossy().replace('\\', "/");

        assert!(rendered.contains("//server/share/dbgraph"));
    }

    fn assert_path_ends_with(path: &Path, expected: &[&str]) {
        let parts = path
            .components()
            .filter_map(|component| match component {
                Component::Normal(part) => Some(part.to_string_lossy().into_owned()),
                Component::RootDir
                | Component::Prefix(_)
                | Component::CurDir
                | Component::ParentDir => None,
            })
            .collect::<Vec<_>>();

        assert!(
            parts.ends_with(
                &expected
                    .iter()
                    .map(|part| (*part).to_owned())
                    .collect::<Vec<_>>()
            ),
            "{parts:?} did not end with {expected:?}"
        );
    }

    struct TempProject {
        root: PathBuf,
    }

    impl TempProject {
        fn new() -> Self {
            let unique = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock should be valid")
                .as_nanos();
            let root = std::env::temp_dir().join(format!(
                "dbgraph-project-test-{}-{unique}",
                std::process::id()
            ));
            fs::create_dir_all(&root).expect("temp root should be created");
            Self { root }
        }
    }

    impl Drop for TempProject {
        fn drop(&mut self) {
            if self.root.exists() {
                fs::remove_dir_all(&self.root).expect("temp root should be removed");
            }
        }
    }
}

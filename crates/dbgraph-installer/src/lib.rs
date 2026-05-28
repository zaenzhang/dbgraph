//! Installer and agent configuration support for `DbGraph`.
//!
//! This crate currently exposes stable instruction fragments. Download logic,
//! target detection, and real agent config writers are implemented in later
//! tasks.

/// Start marker for managed instruction fragments.
pub const DBGRAPH_SECTION_START: &str = "<!-- DBGRAPH_START -->";

/// End marker for managed instruction fragments.
pub const DBGRAPH_SECTION_END: &str = "<!-- DBGRAPH_END -->";

/// Instruction targets supported by the template renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstructionTarget {
    /// Generic `AGENTS.md` fragment.
    AgentsMd,
    /// `CLAUDE.md` fragment.
    ClaudeMd,
    /// Cursor `.mdc` rule fragment.
    CursorRule,
}

impl InstructionTarget {
    /// Returns the default file name for the target.
    #[must_use]
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::AgentsMd => "AGENTS.md.fragment",
            Self::ClaudeMd => "CLAUDE.md.fragment",
            Self::CursorRule => "dbgraph.mdc",
        }
    }

    fn cursor_frontmatter(self) -> &'static str {
        match self {
            Self::CursorRule => {
                "---\ndescription: DbGraph database context rules\nalwaysApply: true\n---\n\n"
            }
            Self::AgentsMd | Self::ClaudeMd => "",
        }
    }
}

/// Render a stable instruction fragment for an agent target.
#[must_use]
pub fn render_instruction_fragment(target: InstructionTarget) -> String {
    format!(
        "{frontmatter}{start}\n## DbGraph\n\n{body}\n{end}\n",
        frontmatter = target.cursor_frontmatter(),
        start = DBGRAPH_SECTION_START,
        body = INSTRUCTION_BODY,
        end = DBGRAPH_SECTION_END
    )
}

/// Render all supported instruction fragments.
#[must_use]
pub fn render_all_instruction_fragments() -> Vec<(InstructionTarget, String)> {
    [
        InstructionTarget::AgentsMd,
        InstructionTarget::ClaudeMd,
        InstructionTarget::CursorRule,
    ]
    .into_iter()
    .map(|target| (target, render_instruction_fragment(target)))
    .collect()
}

const INSTRUCTION_BODY: &str = r#"This project can use DbGraph database context through CLI/MCP tools.

### When To Use DbGraph

Use DbGraph for database-structure questions, especially before editing SQL,
migrations, ORM models, data-access code, or API behavior that depends on
tables, columns, keys, indexes, views, triggers, or query workload.

| Question | Tool |
|---|---|
| "What tables or columns match X?" | `dbgraph_search` |
| "Show me table X" | `dbgraph_table` |
| "What references or depends on X?" | `dbgraph_relations` |
| "What context do I need for this DB task?" | `dbgraph_context` |
| "What could break if this schema changes?" | `dbgraph_impact` |

### Rules

- Do not guess table names, column names, relation directions, or constraint
  behavior when DbGraph context is available. Query DbGraph first.
- Treat explicit database constraints as authoritative. Treat inferred
  relations as hints and label them as inferred.
- DbGraph is read-only for target business databases by default. Do not execute
  DDL, DML, or AI-generated write SQL through DbGraph.
- DbGraph must not store raw business row data by default. Sampling must be
  explicit opt-in and masked according to project configuration.
- Use native file search only for literal source text; use DbGraph for database
  structure, relationships, context, and impact.
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agents_fragment_contains_safety_boundaries() {
        let fragment = render_instruction_fragment(InstructionTarget::AgentsMd);

        assert!(fragment.contains(DBGRAPH_SECTION_START));
        assert!(fragment.contains("Do not guess table names"));
        assert!(fragment.contains("read-only"));
        assert!(fragment.contains("must not store raw business row data"));
    }

    #[test]
    fn cursor_fragment_has_frontmatter() {
        let fragment = render_instruction_fragment(InstructionTarget::CursorRule);

        assert!(fragment.starts_with("---\n"));
        assert!(fragment.contains("alwaysApply: true"));
        assert!(fragment.contains(DBGRAPH_SECTION_END));
    }

    #[test]
    fn all_fragments_have_stable_file_names() {
        let rendered = render_all_instruction_fragments();
        let names = rendered
            .iter()
            .map(|(target, _)| target.file_name())
            .collect::<Vec<_>>();

        assert_eq!(
            names,
            vec!["AGENTS.md.fragment", "CLAUDE.md.fragment", "dbgraph.mdc"]
        );
    }
}

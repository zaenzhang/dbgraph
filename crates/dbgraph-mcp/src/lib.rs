//! MCP server integration for `DbGraph`.
//!
//! The actual stdio server is implemented in a later task.

/// Describes the current implementation status of the MCP crate.
#[must_use]
pub fn crate_status() -> &'static str {
    "mcp skeleton"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_stable() {
        assert_eq!(crate_status(), "mcp skeleton");
    }
}

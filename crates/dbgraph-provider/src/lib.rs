//! Database provider abstractions for `DbGraph`.
//!
//! Provider traits and concrete database integrations are introduced in later
//! tasks. Target business databases must remain read-only by default.

/// Describes the current implementation status of the provider crate.
#[must_use]
pub fn crate_status() -> &'static str {
    "provider skeleton"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_stable() {
        assert_eq!(crate_status(), "provider skeleton");
    }
}

//! Local storage crate for `DbGraph`.
//!
//! `SQLite` migrations and repositories are introduced in later tasks.

/// Describes the current implementation status of the storage crate.
#[must_use]
pub fn crate_status() -> &'static str {
    "storage skeleton"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_stable() {
        assert_eq!(crate_status(), "storage skeleton");
    }
}

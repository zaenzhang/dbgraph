//! Installer and agent configuration support for `DbGraph`.
//!
//! Download logic and agent configuration writers are implemented in later
//! tasks.

/// Describes the current implementation status of the installer crate.
#[must_use]
pub fn crate_status() -> &'static str {
    "installer skeleton"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_stable() {
        assert_eq!(crate_status(), "installer skeleton");
    }
}

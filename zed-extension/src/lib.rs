//! Zed extension that runs Kotlin Navigator for Kotlin and Java.

pub mod launch;
pub mod release;

/// The language server id, as in `extension.toml` and the `lsp` settings.
pub const SERVER_ID: &str = "kotlin-navigator";
/// The GitHub repository whose releases carry the binaries.
pub const REPO: &str = "appolon-dev/kotlin-navigator";
/// The binary's name in a release archive and on `PATH`.
pub const BINARY: &str = "kotlin-navigator";

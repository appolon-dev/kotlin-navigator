//! The command Zed starts: a binary path plus the arguments and environment from the
//! `lsp.kotlin-navigator.binary` settings.

use zed_extension_api::{self as zed, settings::CommandSettings};

/// The binary the user set in `lsp.kotlin-navigator.binary.path`.
pub fn configured_path(binary: &Option<CommandSettings>) -> Option<String> {
    binary.as_ref().and_then(|b| b.path.clone())
}

/// The command for the binary at `path`: arguments and environment come from the settings
/// whichever binary runs, so `binary.env` turns logs on for a downloaded one too.
pub fn command(path: String, binary: Option<CommandSettings>) -> zed::Command {
    let (args, env) = binary.map(|b| (b.arguments.unwrap_or_default(), b.env.unwrap_or_default())).unwrap_or_default();
    let mut env: Vec<(String, String)> = env.into_iter().collect();
    env.sort();
    zed::Command { command: path, args, env }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use zed_extension_api::settings::CommandSettings;

    #[test] fn configured_path_reads_binary_path() {
        let set = Some(CommandSettings { path: Some("/opt/kn".into()), arguments: None, env: None });
        assert_eq!(configured_path(&set), Some("/opt/kn".into()));
        assert_eq!(configured_path(&None), None);
        assert_eq!(configured_path(&Some(CommandSettings { path: None, arguments: None, env: None })), None);
    }
    #[test] fn command_applies_settings_env_and_args() {
        let env = HashMap::from([("KOTLIN_NAVIGATOR_LOG".to_string(), "info".to_string()), ("A".to_string(), "1".to_string())]);
        let c = command("kotlin-navigator-v0.3.0/kotlin-navigator".into(), Some(CommandSettings { path: None, arguments: Some(vec!["--x".into()]), env: Some(env) }));
        assert_eq!(c.command, "kotlin-navigator-v0.3.0/kotlin-navigator");
        assert_eq!(c.args, vec!["--x".to_string()]);
        assert_eq!(c.env, vec![("A".into(), "1".into()), ("KOTLIN_NAVIGATOR_LOG".into(), "info".into())]);
    }
    #[test] fn command_defaults_to_no_args() {
        let c = command("/p".into(), None);
        assert!(c.args.is_empty() && c.env.is_empty());
    }
}

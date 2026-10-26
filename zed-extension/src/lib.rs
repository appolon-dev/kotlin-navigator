//! Zed extension that runs Kotlin Navigator for Kotlin and Java.

pub mod launch;
pub mod release;

use std::fs;

use zed_extension_api::{
    self as zed, LanguageServerId, LanguageServerInstallationStatus as Status, Result, settings::LspSettings,
};

use launch::{command, configured_path};
use release::{Choice, Latest, asset_name, choose, stale_dirs};

/// The language server id, as in `extension.toml` and the `lsp` settings.
pub const SERVER_ID: &str = "kotlin-navigator";
/// The GitHub repository whose releases carry the binaries.
pub const REPO: &str = "appolon-dev/kotlin-navigator";
/// The binary's name in a release archive and on `PATH`.
pub const BINARY: &str = "kotlin-navigator";

struct KotlinNavigator {
    /// The downloaded binary found earlier in this Zed session.
    cached: Option<String>,
}

impl zed::Extension for KotlinNavigator {
    fn new() -> Self {
        KotlinNavigator { cached: None }
    }

    /// The binary from the settings, else from `PATH`, else the downloaded one.
    fn language_server_command(&mut self, id: &LanguageServerId, worktree: &zed::Worktree) -> Result<zed::Command> {
        let binary = LspSettings::for_worktree(SERVER_ID, worktree).ok().and_then(|s| s.binary);
        let path = match configured_path(&binary).or_else(|| worktree.which(BINARY)) {
            Some(path) => path,
            None => self.binary_path(id)?,
        };
        Ok(command(path, binary))
    }
}

impl KotlinNavigator {
    /// The downloaded binary: the one found earlier in the session, the latest release, or
    /// the newest installed copy when the release cannot be had. Paths are relative to the
    /// extension's work directory, where Zed resolves them.
    fn binary_path(&mut self, id: &LanguageServerId) -> Result<String> {
        if let Some(path) = self.cached.as_ref().filter(|p| is_file(p)) {
            return Ok(path.clone());
        }
        let (os, arch) = zed::current_platform();
        let asset = asset_name(os, arch).map_err(|message| failed(id, message))?;
        zed::set_language_server_installation_status(id, &Status::CheckingForUpdate);
        let options = zed::GithubReleaseOptions { require_assets: true, pre_release: false };
        let latest = zed::latest_github_release(REPO, options).map(|release| Latest {
            url: release.assets.into_iter().find(|a| a.name == asset).map(|a| a.download_url),
            tag: release.version,
        });
        let all = dir_names();
        let installed: Vec<String> = all.iter().filter(|d| is_file(&binary_in(d))).cloned().collect();
        let mut choice = choose(latest, &installed, &asset);
        if let Choice::Download { dir, url } = &choice {
            choice = match download(id, dir, url) {
                Ok(()) => {
                    for stale in stale_dirs(&all, dir) {
                        fs::remove_dir_all(stale).ok();
                    }
                    Choice::Use(dir.clone())
                }
                Err(reason) => choose(Err(reason), &installed, &asset),
            };
        }
        let dir = match choice {
            Choice::Use(dir) => dir,
            // `choose` downloads only with a release in hand, and the retry above has none.
            Choice::Download { .. } => unreachable!("a failed download is retried without a release"),
            Choice::Fail(message) => return Err(failed(id, message)),
        };
        zed::set_language_server_installation_status(id, &Status::None);
        let path = binary_in(&dir);
        self.cached = Some(path.clone());
        Ok(path)
    }
}

/// Downloads the release at `url` into `<dir>.partial` and renames it to `dir` only when
/// complete, so an interrupted download never looks installed.
fn download(id: &LanguageServerId, dir: &str, url: &str) -> Result<()> {
    zed::set_language_server_installation_status(id, &Status::Downloading);
    let partial = format!("{dir}.partial");
    fs::remove_dir_all(&partial).ok();
    let done = zed::download_file(url, &partial, zed::DownloadedFileType::GzipTar)
        .and_then(|()| zed::make_file_executable(&binary_in(&partial)))
        .and_then(|()| {
            // `choose` downloads only when `dir` has no binary: whatever is left there goes.
            fs::remove_dir_all(dir).ok();
            fs::rename(&partial, dir).map_err(|e| e.to_string())
        });
    if done.is_err() {
        fs::remove_dir_all(&partial).ok();
    }
    done
}

/// Shows `message` as the server's failed status and returns it as the error.
fn failed(id: &LanguageServerId, message: String) -> String {
    zed::set_language_server_installation_status(id, &Status::Failed(message.clone()));
    message
}

/// Names of the directories in the extension's work directory.
fn dir_names() -> Vec<String> {
    let Ok(entries) = fs::read_dir(".") else { return Vec::new() };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|e| e.file_name().into_string().ok())
        .collect()
}

fn binary_in(dir: &str) -> String {
    format!("{dir}/{BINARY}")
}

fn is_file(path: &str) -> bool {
    fs::metadata(path).is_ok_and(|m| m.is_file())
}

zed::register_extension!(KotlinNavigator);

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn manifest_matches_constants() {
        let manifest = include_str!("../extension.toml");
        assert!(manifest.contains(&format!("[language_servers.{SERVER_ID}]")));
        assert!(manifest.contains(r#"languages = ["Kotlin", "Java"]"#));
        let (owner, name) = REPO.split_once('/').unwrap();
        assert!(manifest.contains(&format!(r#"path = ["{owner}", "{name}", "**"]"#)));
        assert!(manifest.contains(&format!(r#"version = "{}""#, env!("CARGO_PKG_VERSION"))));
    }
}

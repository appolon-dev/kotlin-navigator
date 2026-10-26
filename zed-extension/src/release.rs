//! Which release to run: archive names, version directories and the choice between
//! an installed copy, a download and a failure. No calls into Zed, so it is tested on the host.

use zed_extension_api::{Architecture, Os};

/// Tells the user how to run the navigator without a download.
const HINT: &str = "Put kotlin-navigator on PATH or set lsp.kotlin-navigator.binary.path.";

/// Version directories are named after the release tag: `kotlin-navigator-v0.3.0`.
const PREFIX: &str = "kotlin-navigator-";

/// The release archive for this platform: only macOS on Apple Silicon or Intel has one.
pub fn asset_name(os: Os, arch: Architecture) -> Result<String, String> {
    let target = match (os, arch) {
        (Os::Mac, Architecture::Aarch64) => "aarch64-apple-darwin",
        (Os::Mac, Architecture::X8664) => "x86_64-apple-darwin",
        _ => return Err(format!("Kotlin Navigator: no prebuilt binary for {os:?} {arch:?}. {HINT}")),
    };
    Ok(format!("kotlin-navigator-{target}.tar.gz"))
}

/// The directory a release is unpacked into.
pub fn version_dir(tag: &str) -> String {
    format!("{PREFIX}{tag}")
}

/// `(major, minor, patch)` of a version directory; `None` for any other name, a
/// `.partial` download included.
fn version_of(dir: &str) -> Option<(u32, u32, u32)> {
    let mut parts = dir.strip_prefix(PREFIX)?.strip_prefix('v')?.split('.').map(|p| p.parse::<u32>().ok());
    let version = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(version)
}

/// The newest of the version directories, compared by numbers.
pub fn newest_installed(dirs: &[String]) -> Option<String> {
    dirs.iter().filter_map(|d| version_of(d).map(|v| (v, d))).max().map(|(_, d)| d.clone())
}

/// The directories an update leaves behind: every one of ours but `keep`.
pub fn stale_dirs(dirs: &[String], keep: &str) -> Vec<String> {
    dirs.iter().filter(|d| d.starts_with(PREFIX) && d.as_str() != keep).cloned().collect()
}

/// The latest release and the download URL of this platform's archive, if it has one.
pub struct Latest {
    pub tag: String,
    pub url: Option<String>,
}

/// What to run: an installed version directory, a release to download, or why neither works.
#[derive(Debug, PartialEq, Eq)]
pub enum Choice {
    Use(String),
    Download { dir: String, url: String },
    Fail(String),
}

/// The latest release when it can be had, else the newest installed version. `installed`
/// holds the version directories that contain the binary.
pub fn choose(latest: Result<Latest, String>, installed: &[String], asset: &str) -> Choice {
    let reason = match latest {
        Ok(Latest { tag, url: Some(url) }) => {
            let dir = version_dir(&tag);
            return if installed.contains(&dir) { Choice::Use(dir) } else { Choice::Download { dir, url } };
        }
        Ok(Latest { tag, url: None }) => format!("release {tag} has no {asset}"),
        Err(reason) => reason,
    };
    match newest_installed(installed) {
        Some(dir) => Choice::Use(dir),
        None => Choice::Fail(format!("Kotlin Navigator: could not download the navigator ({reason}). {HINT}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zed_extension_api::{Architecture, Os};

    #[test] fn asset_name_for_apple_silicon() { assert_eq!(asset_name(Os::Mac, Architecture::Aarch64), Ok("kotlin-navigator-aarch64-apple-darwin.tar.gz".into())); }
    #[test] fn asset_name_for_intel() { assert_eq!(asset_name(Os::Mac, Architecture::X8664), Ok("kotlin-navigator-x86_64-apple-darwin.tar.gz".into())); }
    #[test] fn asset_name_rejects_other_platforms() {
        for (os, arch) in [(Os::Linux, Architecture::Aarch64), (Os::Windows, Architecture::X8664), (Os::Mac, Architecture::X86)] {
            let e = asset_name(os, arch).unwrap_err();
            assert!(e.starts_with("Kotlin Navigator: no prebuilt binary for "), "{e}");
            assert!(e.ends_with("Put kotlin-navigator on PATH or set lsp.kotlin-navigator.binary.path."), "{e}");
        }
    }
    #[test] fn version_dir_keeps_the_tag() { assert_eq!(version_dir("v0.3.0"), "kotlin-navigator-v0.3.0"); }
    #[test] fn newest_installed_compares_numbers() { assert_eq!(newest_installed(&dirs(&["kotlin-navigator-v0.9.0", "kotlin-navigator-v0.10.0"])), Some("kotlin-navigator-v0.10.0".into())); }
    #[test] fn newest_installed_skips_foreign_and_partial() {
        assert_eq!(newest_installed(&dirs(&["kotlin-navigator-v0.3.0.partial", "notes", "kotlin-navigator-v0.2.0", "kotlin-navigator-vX"])), Some("kotlin-navigator-v0.2.0".into()));
        assert_eq!(newest_installed(&[]), None);
    }
    #[test] fn stale_dirs_keeps_current_and_foreign() {
        let all = dirs(&["kotlin-navigator-v0.2.0", "kotlin-navigator-v0.3.0", "kotlin-navigator-v0.3.0.partial", "notes"]);
        assert_eq!(stale_dirs(&all, "kotlin-navigator-v0.3.0"), dirs(&["kotlin-navigator-v0.2.0", "kotlin-navigator-v0.3.0.partial"]));
    }
    const ASSET: &str = "kotlin-navigator-aarch64-apple-darwin.tar.gz";
    fn latest(url: Option<&str>) -> Result<Latest, String> { Ok(Latest { tag: "v0.3.0".into(), url: url.map(Into::into) }) }
    #[test] fn choose_uses_installed_latest() { assert_eq!(choose(latest(Some("https://u")), &dirs(&["kotlin-navigator-v0.3.0"]), ASSET), Choice::Use("kotlin-navigator-v0.3.0".into())); }
    #[test] fn choose_downloads_new_version() { assert_eq!(choose(latest(Some("https://u")), &dirs(&["kotlin-navigator-v0.2.0"]), ASSET), Choice::Download { dir: "kotlin-navigator-v0.3.0".into(), url: "https://u".into() }); }
    #[test] fn choose_falls_back_when_offline() { assert_eq!(choose(Err("network down".into()), &dirs(&["kotlin-navigator-v0.2.0"]), ASSET), Choice::Use("kotlin-navigator-v0.2.0".into())); }
    #[test] fn choose_falls_back_when_asset_missing() { assert_eq!(choose(latest(None), &dirs(&["kotlin-navigator-v0.2.0"]), ASSET), Choice::Use("kotlin-navigator-v0.2.0".into())); }
    #[test] fn choose_fails_without_any_install() {
        assert_eq!(choose(Err("network down".into()), &[], ASSET), Choice::Fail("Kotlin Navigator: could not download the navigator (network down). Put kotlin-navigator on PATH or set lsp.kotlin-navigator.binary.path.".into()));
        let Choice::Fail(e) = choose(latest(None), &[], ASSET) else { panic!() };
        assert!(e.contains("(release v0.3.0 has no kotlin-navigator-aarch64-apple-darwin.tar.gz)"), "{e}");
    }
    fn dirs(names: &[&str]) -> Vec<String> { names.iter().map(|s| s.to_string()).collect() }
}

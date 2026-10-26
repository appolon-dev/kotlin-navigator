//! `--bench <root>`: cold indexing statistics for acceptance runs.

use std::path::Path;

use crate::scan;

pub fn run(root: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(root.is_dir(), "not a directory: {}", root.display());
    let (index, stats) = scan::build_index(&[root.to_path_buf()]);
    let report = format!(
        "files: {}\nskipped: {}\ndecls: {}\nparse_error_files: {}\ncold_index_ms: {}\npeak_rss_mb: {}\n",
        stats.files,
        stats.skipped,
        stats.decls,
        stats.error_files,
        stats.elapsed.as_millis(),
        peak_rss_mb()
    );
    drop(index);
    Ok(report)
}

fn peak_rss_mb() -> u64 {
    // SAFETY: getrusage only writes into the zeroed struct we pass.
    let usage = unsafe {
        let mut usage: libc::rusage = std::mem::zeroed();
        libc::getrusage(libc::RUSAGE_SELF, &mut usage);
        usage
    };
    let max_rss = usage.ru_maxrss as u64;
    // macOS reports bytes, Linux kilobytes.
    let bytes = if cfg!(target_os = "macos") { max_rss } else { max_rss * 1024 };
    bytes / (1024 * 1024)
}

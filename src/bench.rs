//! `--bench <root>`: cold indexing statistics and sampled definition lookups for acceptance runs.

use std::path::Path;
use std::time::{Duration, Instant};

use tree_sitter::{Node, Tree};

use crate::index::Index;
use crate::model::Lang;
use crate::resolve::{self, Current};
use crate::{parse, scan};

const SAMPLES: usize = 1000;
const MAX_ATTEMPTS: usize = 10_000;
const SEED: u64 = 42;

pub fn run(root: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(root.is_dir(), "not a directory: {}", root.display());
    let (index, stats) = scan::build_index(&[root.to_path_buf()]);
    let samples = sample_definitions(&index);
    let share = |keep: fn(usize) -> bool| {
        if samples.is_empty() { 0.0 } else { 100.0 * samples.iter().filter(|(n, _)| keep(*n)).count() as f64 / samples.len() as f64 }
    };
    let mut micros: Vec<u128> = samples.iter().map(|(_, t)| t.as_micros()).collect();
    micros.sort_unstable();
    let report = format!(
        "files: {}\nskipped: {}\ndecls: {}\nparse_error_files: {}\ncold_index_ms: {}\npeak_rss_mb: {}\n\
         definition_samples: {}\ndefinition_zero_pct: {:.1}\ndefinition_one_pct: {:.1}\ndefinition_many_pct: {:.1}\n\
         definition_p50_us: {}\ndefinition_p95_us: {}\n",
        stats.files,
        stats.skipped,
        stats.decls,
        stats.error_files,
        stats.elapsed.as_millis(),
        peak_rss_mb(),
        samples.len(),
        share(|n| n == 0),
        share(|n| n == 1),
        share(|n| n > 1),
        percentile(&micros, 50),
        percentile(&micros, 95),
    );
    drop(index);
    Ok(report)
}

/// Go to Definition on random identifiers of random Kotlin files (seeded, so runs compare):
/// the number of targets and the time of each lookup, parsing excluded.
fn sample_definitions(index: &Index) -> Vec<(usize, Duration)> {
    let mut files: Vec<&Path> = index.files().filter(|f| f.lang == Lang::Kotlin).map(|f| f.path.as_path()).collect();
    files.sort();
    let mut rng = SplitMix64(SEED);
    let mut samples = Vec::with_capacity(SAMPLES);
    for _ in 0..MAX_ATTEMPTS {
        if samples.len() == SAMPLES || files.is_empty() {
            break;
        }
        let path = files[rng.below(files.len())];
        let Some(text) = scan::read_source(path) else { continue };
        let Some((tree, _)) = parse::summarize_text(path.to_path_buf(), &text) else { continue };
        let names = identifiers(&tree);
        let (Some(summary), false) = (index.file(path), names.is_empty()) else { continue };
        let byte = names[rng.below(names.len())];
        let started = Instant::now();
        let found = resolve::definition(index, &Current { text: &text, tree: &tree, summary }, byte).len();
        samples.push((found, started.elapsed()));
    }
    samples
}

/// Start bytes of `identifier` nodes outside the package line and imports.
fn identifiers(tree: &Tree) -> Vec<usize> {
    let mut out = Vec::new();
    let mut stack: Vec<Node> = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        match node.kind() {
            "package_header" | "import" => continue,
            "identifier" => out.push(node.start_byte()),
            _ => {}
        }
        let mut cursor = node.walk();
        stack.extend(node.named_children(&mut cursor));
    }
    out.sort_unstable();
    out
}

/// Nearest-rank percentile of sorted values; 0 when there are none.
fn percentile(sorted: &[u128], p: usize) -> u128 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(sorted.len() * p).div_ceil(100).max(1) - 1]
}

struct SplitMix64(u64);

impl SplitMix64 {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
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

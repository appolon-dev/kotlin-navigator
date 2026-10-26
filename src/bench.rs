//! `--bench <root>`: cold indexing statistics and sampled definition lookups for acceptance runs.

use std::path::Path;
use std::time::{Duration, Instant};

use tree_sitter::{Node, Tree};

use crate::cancel::Cancel;
use crate::docs::{OpenDoc, byte_offset};
use crate::index::Index;
use crate::model::Lang;
use crate::resolve::{self, Certainty, Current};
use crate::sources::Sources;
use crate::{highlight, lsp, parse, scan, usages};

const SAMPLES: usize = 1000;
const REFERENCE_SAMPLES: usize = 200;
const MAX_ATTEMPTS: usize = 10_000;
const SEED: u64 = 42;

pub fn run(root: &Path) -> anyhow::Result<String> {
    anyhow::ensure!(root.is_dir(), "not a directory: {}", root.display());
    let (index, stats) = scan::build_index(&[root.to_path_buf()]);
    let (samples, highlights) = sample_definitions(&index);
    let references = sample_references(&index);
    let share = |keep: fn(usize) -> bool| {
        if samples.is_empty() { 0.0 } else { 100.0 * samples.iter().filter(|(n, _)| keep(*n)).count() as f64 / samples.len() as f64 }
    };
    let mut micros: Vec<u128> = samples.iter().map(|(_, t)| t.as_micros()).collect();
    micros.sort_unstable();
    let mut highlight_micros: Vec<u128> = highlights.iter().map(Duration::as_micros).collect();
    highlight_micros.sort_unstable();
    let mut reference_millis: Vec<u128> = references.times.iter().map(Duration::as_millis).collect();
    reference_millis.sort_unstable();
    let probable_pct = if references.found == 0 { 0.0 } else { 100.0 * references.probable as f64 / references.found as f64 };
    let report = format!(
        "files: {}\nskipped: {}\ndecls: {}\nparse_error_files: {}\ncold_index_ms: {}\npeak_rss_mb: {}\n\
         definition_samples: {}\ndefinition_zero_pct: {:.1}\ndefinition_one_pct: {:.1}\ndefinition_many_pct: {:.1}\n\
         definition_p50_us: {}\ndefinition_p95_us: {}\n\
         references_samples: {}\nreferences_p50_ms: {}\nreferences_p95_ms: {}\nreferences_max_ms: {}\nreferences_probable_pct: {:.1}\n\
         highlight_p50_us: {}\nhighlight_p95_us: {}\n",
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
        reference_millis.len(),
        percentile(&reference_millis, 50),
        percentile(&reference_millis, 95),
        reference_millis.last().copied().unwrap_or(0),
        probable_pct,
        percentile(&highlight_micros, 50),
        percentile(&highlight_micros, 95),
    );
    drop(index);
    Ok(report)
}

/// Go to Definition on random identifiers of random Kotlin files (seeded, so runs compare):
/// the number of targets and the time of each lookup, parsing excluded; then the time of
/// the document highlight at the same position.
fn sample_definitions(index: &Index) -> (Vec<(usize, Duration)>, Vec<Duration>) {
    let mut files: Vec<&Path> = index.files().filter(|f| f.lang == Lang::Kotlin).map(|f| f.path.as_path()).collect();
    files.sort();
    let mut rng = SplitMix64(SEED);
    let mut samples = Vec::with_capacity(SAMPLES);
    let mut highlights = Vec::with_capacity(SAMPLES);
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
        let current = Current { text: &text, tree: &tree, summary };
        let found = resolve::definition(index, &current, &Sources::default(), byte).len();
        samples.push((found, started.elapsed()));
        let started = Instant::now();
        highlight::highlight(index, &current, &Sources::default(), byte);
        highlights.push(started.elapsed());
    }
    (samples, highlights)
}

/// Timings of the usage searches and how many of the places they found were `Probable`.
struct ReferenceSamples {
    times: Vec<Duration>,
    found: usize,
    probable: usize,
}

/// Find All References on the name of random declarations of random Kotlin files (seeded),
/// on a pool of the server's size. Each sample is timed as the server does it: a copy of
/// the index, then the search with fresh sources; parsing the current file is excluded.
fn sample_references(index: &Index) -> ReferenceSamples {
    let mut files: Vec<&Path> = index.files().filter(|f| f.lang == Lang::Kotlin).map(|f| f.path.as_path()).collect();
    files.sort();
    let mut out = ReferenceSamples { times: Vec::with_capacity(REFERENCE_SAMPLES), found: 0, probable: 0 };
    let Ok(pool) = rayon::ThreadPoolBuilder::new().num_threads(lsp::REQUEST_THREADS).build() else { return out };
    let mut rng = SplitMix64(SEED);
    pool.install(|| {
        for _ in 0..MAX_ATTEMPTS {
            if out.times.len() == REFERENCE_SAMPLES || files.is_empty() {
                break;
            }
            let path = files[rng.below(files.len())];
            let Some(summary) = index.file(path) else { continue };
            if summary.decls.is_empty() {
                continue;
            }
            let range = summary.decls[rng.below(summary.decls.len())].name_range;
            let Some(text) = scan::read_source(path) else { continue };
            let Some(doc) = OpenDoc::parse(path.to_path_buf(), text, None) else { continue };
            let Some(byte) = byte_offset(&doc.text, range.line, range.start) else { continue };
            let started = Instant::now();
            let copy = Index::clone(index);
            let current = Current::of(&copy, &doc);
            let Ok(places) = usages::references(&copy, &current, &Sources::default(), byte, false, &Cancel::default()) else { continue };
            out.times.push(started.elapsed());
            out.found += places.len();
            out.probable += places.iter().filter(|u| u.certainty == Certainty::Probable).count();
        }
    });
    out
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

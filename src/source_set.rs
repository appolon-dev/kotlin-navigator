//! Gradle source sets taken from the path (`src/<name>/`), and which sets a file can see.

use std::path::{Component, Path};

/// For each source set, the sets its code can use (itself included).
const VISIBLE: &[(&str, &[&str])] = &[
    ("commonMain", &["commonMain"]),
    ("androidMain", &["commonMain", "androidMain"]),
    ("appleMain", &["commonMain", "appleMain"]),
    ("iosMain", &["commonMain", "appleMain", "iosMain"]),
    ("commonTest", &["commonMain", "commonTest"]),
    ("androidHostTest", &["commonMain", "commonTest", "androidMain", "androidHostTest"]),
    ("androidUnitTest", &["commonMain", "commonTest", "androidMain", "androidUnitTest"]),
    ("androidInstrumentedTest", &["commonMain", "commonTest", "androidMain", "androidInstrumentedTest"]),
    ("appleTest", &["commonMain", "commonTest", "appleMain", "appleTest"]),
    ("iosTest", &["commonMain", "commonTest", "appleMain", "iosMain", "iosTest"]),
    ("main", &["main"]),
    ("test", &["main", "test"]),
    ("androidTest", &["main", "androidTest"]),
];

/// The source set of a file: the directory right after the `src` closest to the file
/// whose name is a known set or ends with `Main`/`Test`.
pub fn source_set_of(path: &Path) -> Option<String> {
    let parts: Vec<&str> = path
        .components()
        .map(|c| match c {
            Component::Normal(n) => n.to_str().unwrap_or(""),
            _ => "",
        })
        .collect();
    (0..parts.len().saturating_sub(2)).rev().find_map(|i| {
        let set = parts[i + 1];
        let known = VISIBLE.iter().any(|(s, _)| *s == set) || set.ends_with("Main") || set.ends_with("Test");
        (parts[i] == "src" && known).then(|| set.to_string())
    })
}

/// Whether code in source set `from` can use declarations from `to`. Unknown or missing
/// sets are compatible with everything.
pub fn sees(from: Option<&str>, to: Option<&str>) -> bool {
    let (Some(from), Some(to)) = (from, to) else { return true };
    let Some((_, visible)) = VISIBLE.iter().find(|(s, _)| *s == from) else { return true };
    let to_is_known = VISIBLE.iter().any(|(_, sets)| sets.contains(&to));
    !to_is_known || visible.contains(&to)
}

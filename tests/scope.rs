use kotlin_navigator::parse::summarize_text;
use kotlin_navigator::resolve::scope::{find_local, implicit_receivers, Implicit, LocalKind};
use std::path::PathBuf;

const TEXT: &str = "class Outer(plain: Int, val prop: Int) {\n  val first = plain\n  fun f(p: Repo) {\n    use(later)\n    val (a, b) = pair\n    val made = Repo(1)\n    val typed: Repo? = null\n    for (i in xs) { i }\n    try { } catch (e: Ex) { e }\n    xs.map { it }; xs.map { q -> q }\n    when (val s = g()) { else -> s }\n    fun local() {}\n    val later = 1\n    run { val p = 2; p }\n    plain\n  }\n  companion object { fun c() { x } }\n}\nfun Repo.ext() { y }\n";

fn line_of(text: &str, byte: usize) -> usize { text[..byte].matches('\n').count() }
/// Lines of the locals found for `name` at (line, needle).
fn locals(line: usize, needle: &str, name: &str) -> Vec<(usize, LocalKind, Option<String>, bool)> {
    let (tree, _) = summarize_text(PathBuf::from("/ws/A.kt"), TEXT).unwrap();
    let start: usize = TEXT.split_inclusive('\n').take(line).map(str::len).sum();
    let at = start + TEXT.split_inclusive('\n').nth(line).unwrap().find(needle).unwrap();
    find_local(&tree, TEXT, at, name).into_iter().map(|l| (line_of(TEXT, l.range.start), l.kind, l.type_name, l.from_call)).collect()
}

#[test]
fn parameters_and_locals_before_the_cursor() {
    assert_eq!(locals(5, "Repo", "p"), vec![(2, LocalKind::Value, Some("Repo".into()), false)]);
    assert!(locals(3, "later", "later").is_empty()); // declared below
    assert_eq!(locals(6, "null", "made"), vec![(5, LocalKind::Value, Some("Repo".into()), true)]);
    assert_eq!(locals(7, "{ i", "typed")[0].2, Some("Repo".into()));
    assert_eq!(locals(7, "{ i", "i")[0].0, 7);
    assert_eq!(locals(8, "{ e", "e")[0].0, 8);
    assert_eq!(locals(9, "it", "it"), vec![(9, LocalKind::Value, None, false)]);
    assert_eq!(locals(9, "-> q", "q")[0].0, 9);
    assert_eq!(locals(10, "-> s", "s")[0].0, 10);
    assert_eq!(locals(12, "later", "local")[0].1, LocalKind::Function);
    assert_eq!(locals(5, "Repo", "a")[0].0, 4); // destructuring
}

#[test]
fn nearest_declaration_comes_first() {
    let got: Vec<usize> = locals(13, "p }", "p").into_iter().map(|l| l.0).collect();
    assert_eq!(got, vec![13, 2]);
}

#[test]
fn plain_class_parameters_are_visible_only_in_initializers() {
    assert_eq!(locals(1, "plain", "plain")[0].0, 0);
    assert!(locals(14, "plain", "plain").is_empty());
}

#[test]
fn implicit_receivers_inside_out() {
    let (tree, _) = summarize_text(PathBuf::from("/ws/A.kt"), TEXT).unwrap();
    let name = |r: &Implicit| match r { Implicit::Container(range) => TEXT[range.clone()].to_string(), Implicit::Extension(t) => format!("ext {t}") };
    let at = |needle: &str| TEXT.find(needle).unwrap();
    assert_eq!(implicit_receivers(&tree, TEXT, at("{ x }")).iter().map(name).collect::<Vec<_>>(), vec!["companion", "Outer"]);
    assert_eq!(implicit_receivers(&tree, TEXT, at("{ y }")).iter().map(name).collect::<Vec<_>>(), vec!["ext Repo"]);
}

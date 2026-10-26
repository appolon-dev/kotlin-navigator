use kotlin_navigator::source_set::{sees, source_set_of};
use std::path::Path;

#[test]
fn set_is_the_dir_after_src() {
    assert_eq!(source_set_of(Path::new("/ws/core/src/androidMain/kotlin/a/A.kt")).as_deref(), Some("androidMain"));
    assert_eq!(source_set_of(Path::new("/ws/app/src/main/java/a/A.java")).as_deref(), Some("main"));
    // `src` выше проекта и в пакете не мешают: берётся ближайший к файлу `src/<набор>` с известным или *Main/*Test именем.
    assert_eq!(source_set_of(Path::new("/home/src/proj/m/src/commonTest/kotlin/a/src/A.kt")).as_deref(), Some("commonTest"));
    assert_eq!(source_set_of(Path::new("/ws/Script.kt")), None);
}

#[test]
fn visibility_follows_the_spec_table() {
    assert!(sees(Some("androidMain"), Some("commonMain")));
    assert!(!sees(Some("androidMain"), Some("iosMain")));
    assert!(sees(Some("iosMain"), Some("appleMain")));
    assert!(!sees(Some("commonMain"), Some("androidMain")));
    assert!(sees(Some("androidHostTest"), Some("commonTest")));
    assert!(sees(Some("androidHostTest"), Some("androidHostTest")));
    assert!(!sees(Some("commonTest"), Some("androidMain")));
    assert!(sees(Some("test"), Some("main")));
    assert!(sees(Some("jvmMain"), Some("iosMain")));  // набора нет в таблице: видит всех
    assert!(sees(Some("commonMain"), Some("jvmMain"))); // и все видят его
    assert!(sees(None, Some("iosMain")));
}

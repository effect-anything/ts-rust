use super::*;
use crate::ext::spanmap_ext::MapperInfo;
use std::cell::RefCell;

fn map_with(deps: &[&str]) -> SpanMap {
    crate::spanmap::new(&[]).with_mapper_info(MapperInfo {
        dependencies: deps.iter().map(|d| (*d).to_owned()).collect(),
        ..Default::default()
    })
}

#[test]
fn salt_is_zero_until_a_dependency_changes_and_unique_per_state() {
    let disk = RefCell::new(HashMap::from([("/t1/m.sts".to_owned(), 10u128)]));
    let state = |p: &str| disk.borrow().get(p).copied().unwrap_or(0);
    let path = "/t1/a.sts";
    assert_eq!(salt_with(path, &state), 0, "never transformed");
    record_with(path, Some(&map_with(&["/t1/m.sts"])), &state);
    assert_eq!(salt_with(path, &state), 0, "baseline: Go's key");
    disk.borrow_mut().insert("/t1/m.sts".into(), 11);
    let edited = salt_with(path, &state);
    assert_ne!(edited, 0);
    // The edit made the macro module import another one.
    record_with(path, Some(&map_with(&["/t1/m.sts", "/t1/n.sts"])), &state);
    disk.borrow_mut().insert("/t1/n.sts".into(), 5);
    let again = salt_with(path, &state);
    assert_ne!(again, 0);
    assert_ne!(again, edited);
    disk.borrow_mut().insert("/t1/n.sts".into(), 6);
    assert_ne!(salt_with(path, &state), again, "a transitive edit counts");
    // Back to the baseline text: the baseline entry is right again.
    disk.borrow_mut().insert("/t1/m.sts".into(), 10);
    assert_eq!(salt_with(path, &state), 0);
}

#[test]
fn files_without_dependencies_are_not_recorded() {
    let state = |_: &str| 1u128;
    record_with("/t2/plain.sts", Some(&map_with(&[])), &state);
    assert!(!records().lock().unwrap().contains_key("/t2/plain.sts"));
    assert_eq!(salt_with("/t2/plain.sts", &state), 0);
}

#[test]
fn disk_state_reads_the_file() {
    let dir = std::env::temp_dir().join(format!("mapper-deps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("m.sts");
    std::fs::write(&file, "a").unwrap();
    let path = file.to_string_lossy().into_owned();
    let first = disk_state(&path);
    std::fs::write(&file, "b").unwrap();
    assert_ne!(first, disk_state(&path));
    assert_eq!(disk_state(&format!("{path}.missing")), 0);
    let _ = std::fs::remove_dir_all(dir);
}

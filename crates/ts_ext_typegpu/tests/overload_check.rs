//! Check and LSP see the author's `+`. This locks the type that produces.

use std::process::Command;

/// The debug tsc (the cargo `tsgo` binary): `TSC_BIN` when set, else
/// `<target>/debug/tsgo`, where the target directory is `CARGO_TARGET_DIR`,
/// else `cargo metadata`'s `target_directory`, else `<root>/target`.
fn tsc() -> std::path::PathBuf {
    use std::path::PathBuf;
    if let Some(bin) = std::env::var_os("TSC_BIN").filter(|bin| !bin.is_empty()) {
        return PathBuf::from(bin);
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .map(PathBuf::from)
        .expect("workspace root");
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .filter(|dir| !dir.is_empty())
        .map(|dir| root.join(dir))
        .or_else(|| metadata_target_dir(&root))
        .unwrap_or_else(|| root.join("target"));
    target
        .join("debug")
        .join(format!("tsgo{}", std::env::consts::EXE_SUFFIX))
}

/// `target_directory` from `cargo metadata`, if cargo runs and says.
fn metadata_target_dir(root: &std::path::Path) -> Option<std::path::PathBuf> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args(["metadata", "--format-version", "1", "--no-deps"])
        .current_dir(root)
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let key = "\"target_directory\":\"";
    let start = text.find(key)? + key.len();
    let end = start + text[start..].find('"')?;
    Some(std::path::PathBuf::from(
        text[start..end].replace("\\\\", "\\"),
    ))
}

#[test]
fn vec2f_plus_is_vec2f_and_a_number_is_an_error() {
    let bin = tsc();
    assert!(
        bin.is_file(),
        "tsc (the cargo `tsgo` binary) missing at {}: build it with `cargo build -p ts_goport --bin tsgo`, or set TSC_BIN",
        bin.display()
    );
    let fixture =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overload");
    let output = Command::new(&bin)
        .args(["--noEmit", "--pretty", "false", "-p"])
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run tsc (the cargo `tsgo` binary): {err}"));
    let report = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        report.contains("Type 'Vec2f' is not assignable to type 'number'"),
        "a + b must be Vec2f, not number:\n{report}"
    );
    assert!(
        report.contains("Operator '+' cannot be applied to types 'Vec2f' and 'number'"),
        "Vec2f + number must be rejected:\n{report}"
    );
    assert!(
        !report.contains("cannot be applied to types 'Vec2f' and 'Vec2f'")
            && !report.contains("cannot be applied to types 'number' and 'Vec2f'"),
        "a legal overload was rejected:\n{report}"
    );
    let errors = report.matches("error TS").count();
    assert_eq!(
        errors, 2,
        "expected the two intentional errors only:\n{report}"
    );
}

//! Check and LSP see the author's `+`. This locks the type that produces.

use std::process::Command;

fn tsgo() -> std::path::PathBuf {
    let mut path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.pop();
    path.pop();
    path.push("target/debug/tsgo");
    path
}

#[test]
fn vec2f_plus_is_vec2f_and_a_number_is_an_error() {
    let bin = tsgo();
    if !bin.is_file() {
        panic!("tsgo missing at {}", bin.display());
    }
    let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/overload");
    let output = Command::new(&bin)
        .args(["--noEmit", "--pretty", "false", "-p"])
        .arg(&fixture)
        .output()
        .unwrap_or_else(|err| panic!("run tsgo: {err}"));
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
    assert_eq!(errors, 2, "expected the two intentional errors only:\n{report}");
}

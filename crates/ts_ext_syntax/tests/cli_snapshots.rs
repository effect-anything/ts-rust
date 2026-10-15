//! `tsc-ext` output snapshots. Each `tests/fixtures/*.{sts,stsx,ts}` names
//! its command on the first line (`// tsc-ext: expand --trace`); the file
//! is appended and the command runs in `tests/fixtures`. The exit code,
//! stdout and stderr must equal `<fixture>.out`. `TSC_EXT_UPDATE=1`
//! rewrites the `.out` files.

use std::path::Path;
use std::process::Command;

const BIN: &str = env!("CARGO_BIN_EXE_tsc-ext");

fn run(dir: &Path, args: &[String], stdin: Option<&str>) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(BIN)
        .args(args)
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .env("TSC_EXT_NO_PREWARM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin.unwrap_or("").as_bytes()).unwrap();
    drop(input);
    let out = child.wait_with_output().unwrap();
    format!(
        "exit: {}\n--- stdout\n{}\n--- stderr\n{}",
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr),
    )
}

fn check(expected: &Path, actual: &str, failures: &mut Vec<String>) {
    if std::env::var_os("TSC_EXT_UPDATE").is_some() {
        std::fs::write(expected, actual).unwrap();
        return;
    }
    let want = std::fs::read_to_string(expected).unwrap_or_default();
    if want != actual {
        failures.push(format!(
            "{} differs (TSC_EXT_UPDATE=1 to accept):\n{actual}",
            expected.display()
        ));
    }
}

#[test]
fn fixtures_match_their_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut failures = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|ext| ext.to_str()),
                Some("sts" | "stsx" | "ts")
            )
        })
        .collect();
    entries.sort();
    assert!(!entries.is_empty());
    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap();
        let command = text
            .lines()
            .next()
            .and_then(|line| line.strip_prefix("// tsc-ext: "))
            .unwrap_or_else(|| {
                panic!(
                    "{}: first line must be `// tsc-ext: <command>`",
                    path.display()
                )
            });
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let mut args: Vec<String> = command.split_whitespace().map(str::to_owned).collect();
        args.push(name.clone());
        let actual = run(&dir, &args, None);
        check(&dir.join(format!("{name}.out")), &actual, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// `eval` cases: (snapshot name, arguments, stdin).
const EVAL: &[(&str, &[&str], &str)] = &[
    ("eval_arg", &["eval", "const a = 1;"], ""),
    (
        "eval_stdin_jsx",
        &["eval", "--jsx", "-"],
        "const v = <div>{1}</div>;\n",
    ),
    (
        "eval_ts_bundle",
        &[
            "eval", "--lang", "ts", "--stage", "bundle", "--only", "tsover",
        ],
        "const f = (a: number, b: number) => {\n  \"use gpu\";\n  return a * b;\n};\n",
    ),
    ("eval_bad_lang", &["eval", "--lang", "rs", "x"], ""),
];

#[test]
fn eval_matches_its_snapshots() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let mut failures = Vec::new();
    for (name, args, stdin) in EVAL {
        let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
        let actual = run(&dir, &args, Some(stdin));
        check(&dir.join(format!("{name}.out")), &actual, &mut failures);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Every command answers `--help` with exit 0 and its examples, and
/// `help <command>` prints the same text.
#[test]
fn every_command_has_help() {
    let dir = std::env::temp_dir();
    for command in ["expand", "eval", "explain", "cheatsheet"] {
        let out = Command::new(BIN)
            .args([command, "--help"])
            .current_dir(&dir)
            .output()
            .unwrap();
        assert!(out.status.success(), "{command}");
        let text = String::from_utf8(out.stdout).unwrap();
        assert!(
            text.contains("EXAMPLES") && text.contains("EXIT CODES"),
            "{command}:\n{text}"
        );
        let again = Command::new(BIN).args(["help", command]).output().unwrap();
        assert_eq!(String::from_utf8(again.stdout).unwrap(), text);
    }
    for unknown in ["frobnicate", "sweetener"] {
        let out = Command::new(BIN)
            .args([unknown, "--expand", "a.sts"])
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(2), "{unknown}");
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(
            stderr.starts_with(&format!("tsc-ext: unknown command {unknown}")),
            "{stderr}"
        );
    }
    let out = Command::new(BIN)
        .args(["explain", "SWR9999"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let out = Command::new(BIN)
        .args(["explain", "4001"])
        .output()
        .unwrap();
    assert!(String::from_utf8(out.stdout).unwrap().contains("SWR4001"));
}

//! One tsconfig option, every reader. The bundler output: the pipeline
//! behind the Node API `Transformer` (`TransformSession::from_options` over
//! the tsconfig's options), the Node API `Project` (`project::open`, then
//! `get_file`), and `tsc-ext expand --stage bundle` must print the same
//! text, and flipping `typegpu.autoNaming` must change it. What tsc checks:
//! the mapper's `transform` (after `openProject` with those options) and
//! `tsc-ext expand` (stage tsc) must agree, with no `TypeGPU` rewrite.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use ts_ext_syntax::TransformSession;
use ts_ext_syntax::config::{Options, read_tsconfig_options};
use ts_ext_syntax::project;

const BIN: &str = env!("CARGO_BIN_EXE_tsc-ext");

const SOURCE: &str = "import tgpu from \"typegpu\";\n\
export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n\
export const pair = twice(1);\n\
export const layout = tgpu.bindGroupLayout({});\n";

fn write_project(dir: &Path, auto_naming: bool) -> String {
    std::fs::create_dir_all(dir).unwrap();
    let options = format!(r#"{{"typegpu":{{"autoNaming":{auto_naming}}}}}"#);
    std::fs::write(
        dir.join("tsconfig.json"),
        format!(
            r#"{{"contentMappers":[{{"package":"@effect-x/tsc","extensions":[".sts"],"options":{options}}}]}}"#
        ),
    )
    .unwrap();
    std::fs::write(dir.join("a.sts"), SOURCE).unwrap();
    options
}

fn cli(file: &Path, stage: &str) -> String {
    let out = Command::new(BIN)
        .args(["expand", "--stage", stage, "--color", "never"])
        .arg(file)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

fn send(stdin: &mut impl Write, body: &str) {
    write!(stdin, "Content-Length: {}\r\n\r\n{body}", body.len()).unwrap();
    stdin.flush().unwrap();
}

fn receive(stdout: &mut impl BufRead) -> serde_json::Value {
    let mut length = 0;
    loop {
        let mut line = String::new();
        stdout.read_line(&mut line).unwrap();
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(value) = line.strip_prefix("Content-Length: ") {
            length = value.parse().unwrap();
        }
    }
    let mut body = vec![0; length];
    stdout.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

/// The mapper's `transform` (what tsc checks) text under the project's options.
fn mapper(dir: &Path, file: &Path, options: &str) -> String {
    let mut child = Command::new(BIN)
        .env("TSC_EXT_NO_PREWARM", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let config = serde_json::to_string(&dir.join("tsconfig.json")).unwrap();
    let file = serde_json::to_string(file).unwrap();
    let content = serde_json::to_string(SOURCE).unwrap();
    send(
        &mut stdin,
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
    );
    receive(&mut stdout);
    send(
        &mut stdin,
        &format!(
            r#"{{"jsonrpc":"2.0","id":2,"method":"openProject","params":{{"configFileName":{config},"projectHandle":"p","options":{options}}}}}"#
        ),
    );
    let opened = receive(&mut stdout);
    assert!(
        opened["result"].get("optionDiagnostics").is_none(),
        "{opened}"
    );
    send(
        &mut stdin,
        &format!(
            r#"{{"jsonrpc":"2.0","id":4,"method":"transform","params":{{"projectHandle":"p","fileName":{file},"content":{content}}}}}"#
        ),
    );
    let checked = receive(&mut stdout)["result"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    drop(stdin);
    child.wait().unwrap();
    checked
}

#[test]
fn a_typegpu_option_changes_api_project_and_cli_output_alike() {
    let root = std::env::temp_dir().join(format!("tsc-ext-options-{}", std::process::id()));
    let mut outputs = Vec::new();
    for auto_naming in [true, false] {
        let dir = root.join(auto_naming.to_string());
        let options = write_project(&dir, auto_naming);
        let dir = std::fs::canonicalize(&dir).unwrap();
        let tsconfig = dir.join("tsconfig.json");
        let file = dir.join("a.sts");
        let api = TransformSession::from_options(&read_tsconfig_options(&tsconfig).unwrap())
            .transform(file.to_str().unwrap(), SOURCE)
            .code;
        let project = project::open(&tsconfig, Options::default(), "{}").unwrap();
        let answer: serde_json::Value =
            serde_json::from_str(&project.get_file(file.to_str().unwrap(), None).unwrap()).unwrap();
        project::close(&project, "{}");
        assert_eq!(
            answer["code"].as_str().unwrap(),
            api,
            "Project vs Transformer, autoNaming {auto_naming}"
        );
        assert_eq!(
            cli(&file, "bundle"),
            api,
            "expand --stage bundle vs Transformer, autoNaming {auto_naming}"
        );
        // What tsc type-checks: no TypeGPU rewrite (tsc keeps the author's
        // operators and does not emit `.sts`), so the same under both.
        let checked = mapper(&dir, &file, &options);
        assert_eq!(checked, cli(&file, "tsc"), "mapper vs expand (stage tsc)");
        assert!(!checked.contains("__TYPEGPU"), "{checked}");
        assert!(api.contains("[1, 1]"), "{api}");
        outputs.push(api);
    }
    let _ = std::fs::remove_dir_all(&root);
    assert_ne!(outputs[0], outputs[1], "autoNaming had no effect");
}

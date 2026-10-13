//! One tsconfig option, three readers: the content mapper (`openProject`
//! options, then `plugin`), `tsc-ext sweetener --expand --build`, and the
//! pipeline behind the Node API (`TransformSession::from_options` over the
//! tsconfig's options, as `node.rs` builds it). Each must print the same
//! text, and flipping `typegpu.autoNaming` must change that text.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use ts_ext_syntax::TransformSession;
use ts_ext_syntax::config::read_tsconfig_options;

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

fn cli(file: &Path, build: bool) -> String {
    let out = Command::new(BIN)
        .args(["sweetener", "--expand"])
        .arg(file)
        .args(if build { &["--build"][..] } else { &[] })
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

/// The mapper's `plugin` (bundler) and `transform` (what tsc checks) texts.
fn mapper(dir: &Path, file: &Path, options: &str) -> (String, String) {
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
            r#"{{"jsonrpc":"2.0","id":3,"method":"plugin","params":{{"projectHandle":"p","fileName":{file},"content":{content}}}}}"#
        ),
    );
    let built = receive(&mut stdout)["result"]["text"]
        .as_str()
        .unwrap()
        .to_owned();
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
    (built, checked)
}

#[test]
fn a_typegpu_option_changes_mapper_cli_and_api_output_alike() {
    let root = std::env::temp_dir().join(format!("tsc-ext-options-{}", std::process::id()));
    let mut outputs = Vec::new();
    for auto_naming in [true, false] {
        let dir = root.join(auto_naming.to_string());
        let options = write_project(&dir, auto_naming);
        let file = dir.join("a.sts");
        let api = TransformSession::from_options(
            &read_tsconfig_options(&dir.join("tsconfig.json")).unwrap(),
        )
        .transform(file.to_str().unwrap(), SOURCE)
        .code;
        assert_eq!(
            cli(&file, true),
            api,
            "CLI vs API, autoNaming {auto_naming}"
        );
        let (built, checked) = mapper(&dir, &file, &options);
        assert_eq!(built, api, "mapper vs API, autoNaming {auto_naming}");
        // What tsc type-checks: no TypeGPU rewrite (tsc keeps the author's
        // operators and does not emit `.sts`), so the same under both.
        assert_eq!(checked, cli(&file, false), "mapper vs CLI check view");
        assert!(!checked.contains("__TYPEGPU"), "{checked}");
        assert!(api.contains("[1, 1]"), "{api}");
        outputs.push(api);
    }
    let _ = std::fs::remove_dir_all(&root);
    assert_ne!(outputs[0], outputs[1], "autoNaming had no effect");
}

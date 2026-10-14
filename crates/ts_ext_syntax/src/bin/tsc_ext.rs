//! `tsc-ext`: the content mapper, and the commands that show what it does.
//!
//! With no arguments (or the hidden `mapper` command) this process speaks
//! the content-mapper protocol on stdin/stdout; a tsconfig `contentMappers`
//! entry for `@effect-x/tsc` starts it. `tsgo` does not link this binary.
//! Run by hand in a terminal with no arguments, it prints the help instead.
//!
//! Every other command (`expand`, `eval`, `explain`, `cheatsheet`, `help`)
//! is [`ts_ext_syntax::cli::run`].

use std::collections::HashMap;
use std::env;
use std::io::{self, IsTerminal, Read, Write};
use std::process::ExitCode;

use ts_ext_syntax::cli;
use ts_ext_syntax::config::{self, Options};
use ts_ext_syntax::protocol::{self, TransformResponse};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None if io::stdin().is_terminal() => {
            eprint!("{}", cli::HELP);
            ExitCode::from(cli::EXIT_USAGE)
        }
        None => serve(),
        Some("mapper") if args.len() == 1 => serve(),
        _ => ExitCode::from(cli::run(&args)),
    }
}

/// Per-project options from `openProject`, by project handle.
type Projects = HashMap<String, Options>;

fn serve() -> ExitCode {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout().lock();
    let mut projects = Projects::new();
    loop {
        let message = match read_message(&mut input) {
            Ok(Some(message)) => message,
            Ok(None) => return ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("tsc-ext: {err}");
                return ExitCode::from(1);
            }
        };
        let response = handle(&message, &mut projects);
        if let Err(err) = write_message(&mut stdout, &response) {
            eprintln!("tsc-ext: {err}");
            return ExitCode::from(1);
        }
    }
}

fn handle(message: &str, projects: &mut Projects) -> String {
    let method = json_string(message, "method").unwrap_or_default();
    let id = json_raw(message, "id");
    let params = json_object(message, "params");
    let result = match method.as_str() {
        "initialize" => r#"{"positionEncoding":"utf-8","diagnosticSource":"tsc-ext"}"#.to_owned(),
        "openProject" => {
            // Expand the project's files in the background so the first
            // transforms (and the first hover) find them in the memo.
            let config = json_string(&params, "configFileName").unwrap_or_default();
            let handle = json_string(&params, "projectHandle").unwrap_or_default();
            let options = json_raw(&params, "options")
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
                .unwrap_or(serde_json::Value::Null);
            let (options, errors) = Options::from_json(&options);
            projects.insert(handle, options);
            let _ = ts_ext_syntax::prewarm::prewarm_in_background(&config);
            let diagnostics: Vec<String> = errors
                .iter()
                .map(|error| {
                    let path: Vec<String> =
                        error.path.iter().map(|part| json_encode(part)).collect();
                    format!(
                        r#"{{"path":[{}],"messageText":{},"code":{OPTION_ERROR}}}"#,
                        path.join(","),
                        json_encode(&format!("{}: {error}", config::PACKAGE)),
                    )
                })
                .collect();
            if diagnostics.is_empty() {
                r#"{"configIdentity":""}"#.to_owned()
            } else {
                format!(
                    r#"{{"configIdentity":"","optionDiagnostics":[{}]}}"#,
                    diagnostics.join(",")
                )
            }
        }
        "closeProject" => {
            if let Some(handle) = json_string(&params, "projectHandle") {
                projects.remove(&handle);
            }
            "{}".to_owned()
        }
        "transform" => {
            let file_name = json_string(&params, "fileName").unwrap_or_default();
            if let Some(message) = ts_ext_syntax::unsupported_source(&file_name) {
                return error_response(id.as_deref(), -32602, &message);
            }
            let content = json_string(&params, "content").unwrap_or_default();
            encode_transform(&protocol::transform_file(&file_name, &content))
        }
        _ => {
            return error_response(id.as_deref(), -32601, &format!("unknown method {method}"));
        }
    };
    match id {
        Some(id) => format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{result}}}"#),
        None => format!(r#"{{"jsonrpc":"2.0","result":{result}}}"#),
    }
}

/// `optionDiagnostics` code for an invalid tsc-ext option.
const OPTION_ERROR: i32 = 9001;

fn encode_transform(response: &TransformResponse) -> String {
    let mut diagnostics = String::from("[");
    for (index, diagnostic) in response.diagnostics.iter().enumerate() {
        if index > 0 {
            diagnostics.push(',');
        }
        let related: Vec<String> = diagnostic
            .related
            .iter()
            .map(|(message, start, length)| {
                format!(
                    r#"{{"messageText":{},"start":{start},"length":{length}}}"#,
                    json_encode(message)
                )
            })
            .collect();
        diagnostics.push_str(&format!(
            r#"{{"messageText":{},"start":{},"length":{},"code":{},"category":{},"relatedInformation":[{}]}}"#,
            json_encode(&diagnostic.message_text),
            diagnostic.start,
            diagnostic.length,
            diagnostic.code,
            diagnostic.category,
            related.join(","),
        ));
    }
    diagnostics.push(']');
    let mappings = if response.mappings == "[]" {
        String::new()
    } else {
        format!(r#""mappings":{},"#, response.mappings)
    };
    let diagnostics = if response.diagnostics.is_empty() {
        String::new()
    } else {
        format!(r#""diagnostics":{diagnostics},"#)
    };
    let expansions = if response.expansions.is_empty() {
        String::new()
    } else {
        let items: Vec<String> = response
            .expansions
            .iter()
            .map(|item| {
                format!(
                    r#"{{"virtualStart":{},"virtualLength":{},"originalStart":{},"originalLength":{},"label":{}}}"#,
                    item.virtual_start,
                    item.virtual_length,
                    item.original_start,
                    item.original_length,
                    json_encode(&item.label),
                )
            })
            .collect();
        format!(r#""expansions":[{}],"#, items.join(","))
    };
    format!(
        r#"{{"text":{},"extension":{},{mappings}{diagnostics}{expansions}}}"#,
        json_encode(&response.text),
        json_encode(response.extension),
    )
    .replace(",}", "}")
}

fn error_response(id: Option<&str>, code: i32, message: &str) -> String {
    let id = id.unwrap_or("null");
    format!(
        r#"{{"jsonrpc":"2.0","id":{id},"error":{{"code":{code},"message":{}}}}}"#,
        json_encode(message)
    )
}

fn read_message(input: &mut impl Read) -> io::Result<Option<String>> {
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        if input.read(&mut byte)? == 0 {
            return if header.is_empty() {
                Ok(None)
            } else {
                Err(io::Error::new(io::ErrorKind::UnexpectedEof, "short header"))
            };
        }
        header.push(byte[0]);
        if header.ends_with(b"\r\n\r\n") {
            break;
        }
    }
    let header = String::from_utf8_lossy(&header);
    let length = header
        .lines()
        .find_map(|line| line.strip_prefix("Content-Length: "))
        .and_then(|value| value.trim().parse::<usize>().ok())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Content-Length"))?;
    let mut body = vec![0u8; length];
    input.read_exact(&mut body)?;
    String::from_utf8(body)
        .map(Some)
        .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))
}

fn write_message(output: &mut impl Write, body: &str) -> io::Result<()> {
    write!(output, "Content-Length: {}\r\n\r\n{body}", body.len())?;
    output.flush()
}

fn json_string(object: &str, key: &str) -> Option<String> {
    let raw = json_raw(object, key)?;
    if raw.starts_with('"') {
        Some(json_decode(&raw))
    } else {
        None
    }
}

fn json_object(object: &str, key: &str) -> String {
    json_raw(object, key).unwrap_or_else(|| "{}".to_owned())
}

fn json_raw(object: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = object.find(&needle)? + needle.len();
    let rest = object[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    Some(take_value(rest))
}

fn take_value(input: &str) -> String {
    let bytes = input.as_bytes();
    if bytes.first() == Some(&b'"') {
        let mut escaped = false;
        for (index, byte) in bytes.iter().enumerate().skip(1) {
            if escaped {
                escaped = false;
                continue;
            }
            if *byte == b'\\' {
                escaped = true;
                continue;
            }
            if *byte == b'"' {
                return input[..=index].to_owned();
            }
        }
        return input.to_owned();
    }
    if bytes.first() == Some(&b'{') || bytes.first() == Some(&b'[') {
        let open = bytes[0];
        let close = if open == b'{' { b'}' } else { b']' };
        let mut depth = 0i32;
        let mut in_string = false;
        let mut escaped = false;
        for (index, byte) in bytes.iter().enumerate() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if *byte == b'\\' {
                    escaped = true;
                } else if *byte == b'"' {
                    in_string = false;
                }
                continue;
            }
            match *byte {
                b'"' => in_string = true,
                value if value == open => depth += 1,
                value if value == close => {
                    depth -= 1;
                    if depth == 0 {
                        return input[..=index].to_owned();
                    }
                }
                _ => {}
            }
        }
    }
    input
        .split([',', '}'])
        .next()
        .unwrap_or(input)
        .trim()
        .to_owned()
}

fn json_decode(quoted: &str) -> String {
    let inner = quoted.trim().trim_matches('"');
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex = chars.by_ref().take(4).collect::<String>();
                if let Ok(value) = u32::from_str_radix(&hex, 16)
                    && let Some(decoded) = char::from_u32(value)
                {
                    out.push(decoded);
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

fn json_encode(value: &str) -> String {
    let mut out = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            ch if ch.is_control() => out.push_str(&format!("\\u{:04x}", u32::from(ch))),
            ch => out.push(ch),
        }
    }
    out.push('"');
    out
}

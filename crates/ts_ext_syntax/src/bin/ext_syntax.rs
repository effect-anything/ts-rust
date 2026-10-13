//! Content mapper and standalone transform.
//!
//! With no arguments this process speaks the content-mapper protocol on
//! stdin/stdout. `ext-syntax transform <file>` writes the virtual
//! TypeScript and the span-map JSON beside it. `tsgo` does not link this
//! binary; a tsconfig `contentMappers` entry starts it.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::process::ExitCode;

use ts_ext_syntax::protocol::{self, TransformResponse};

fn main() -> ExitCode {
    let mut args = env::args().skip(1);
    match args.next().as_deref() {
        None => serve(),
        Some("transform") => transform_cli(args.next()),
        Some("help") | Some("-h") | Some("--help") => {
            eprintln!(
                "ext-syntax\n  (no args)          content mapper on stdin/stdout\n  transform <file>   write <file>.virtual.ts and <file>.mappings.json"
            );
            ExitCode::SUCCESS
        }
        Some(other) => {
            eprintln!("unknown command {other}; try --help");
            ExitCode::from(2)
        }
    }
}

fn transform_cli(path: Option<String>) -> ExitCode {
    let Some(path) = path else {
        eprintln!("usage: ext-syntax transform <file>");
        return ExitCode::from(2);
    };
    let content = match fs::read_to_string(&path) {
        Ok(content) => content,
        Err(err) => {
            eprintln!("{path}: {err}");
            return ExitCode::from(1);
        }
    };
    let response = protocol::transform_file(&path, &content);
    for diagnostic in &response.diagnostics {
        let (line, col) = line_col(&content, diagnostic.start);
        let level = if diagnostic.category == 0 {
            "warning"
        } else {
            "error"
        };
        eprintln!(
            "{path}({line},{col}): {level} {}",
            report_message(diagnostic.code, &diagnostic.message_text)
        );
        for (message, start, _) in &diagnostic.related {
            let (line, col) = line_col(&content, *start);
            eprintln!("  {path}({line},{col}): {message}");
        }
    }
    let virtual_path = format!("{path}.virtual{}", response.extension);
    let map_path = format!("{path}.mappings.json");
    if let Err(err) = fs::write(&virtual_path, &response.text) {
        eprintln!("{virtual_path}: {err}");
        return ExitCode::from(1);
    }
    if let Err(err) = fs::write(&map_path, &response.mappings) {
        eprintln!("{map_path}: {err}");
        return ExitCode::from(1);
    }
    eprintln!("wrote {virtual_path}");
    ExitCode::SUCCESS
}

/// `file.sts(line,col): error SWRxxxx: message`. Sweetener diagnostics
/// already start with `SWR`. A numeric code with no prefix is printed the
/// same way. A `TS` prefix is a TypeScript diagnostic and is left as written.
fn report_message(code: i32, message: &str) -> String {
    if message.starts_with("SWR") || message.starts_with("TS") || code == 0 {
        message.to_owned()
    } else {
        format!("SWR{code}: {message}")
    }
}

fn line_col(content: &str, start: i32) -> (usize, usize) {
    let start = usize::try_from(start).unwrap_or(0).min(content.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for ch in content[..start].chars() {
        if ch == '\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn serve() -> ExitCode {
    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut stdout = io::stdout().lock();
    loop {
        let message = match read_message(&mut input) {
            Ok(Some(message)) => message,
            Ok(None) => return ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("ext-syntax: {err}");
                return ExitCode::from(1);
            }
        };
        let response = handle(&message);
        if let Err(err) = write_message(&mut stdout, &response) {
            eprintln!("ext-syntax: {err}");
            return ExitCode::from(1);
        }
    }
}

fn handle(message: &str) -> String {
    let method = json_string(message, "method").unwrap_or_default();
    let id = json_raw(message, "id");
    let params = json_object(message, "params");
    let result = match method.as_str() {
        "initialize" => {
            r#"{"positionEncoding":"utf-8","diagnosticSource":"ext-syntax"}"#.to_owned()
        }
        "openProject" => r#"{"configIdentity":""}"#.to_owned(),
        "closeProject" => "{}".to_owned(),
        "transform" => {
            let file_name = json_string(&params, "fileName").unwrap_or_default();
            let content = json_string(&params, "content").unwrap_or_default();
            encode_transform(&protocol::transform_file(&file_name, &content))
        }
        "plugin" => {
            let file_name = json_string(&params, "fileName").unwrap_or_default();
            let content = json_string(&params, "content").unwrap_or_default();
            let transformed =
                ts_ext_syntax::TransformSession::new().transform(&file_name, &content);
            encode_plugin(&transformed)
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

fn encode_plugin(value: &ts_ext_syntax::Transformed) -> String {
    format!(
        r#"{{"text":{},"extension":{},"mappings":{},"sourceMap":{},"changed":{}}}"#,
        json_encode(&value.code),
        json_encode(value.extension),
        json_encode(&value.mappings),
        json_encode(&value.source_map),
        value.changed,
    )
}

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
    format!(
        r#"{{"text":{},"extension":{},{mappings}{diagnostics}}}"#,
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

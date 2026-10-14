//! `expand` and `eval`: what a file becomes at a stage, with diagnostics,
//! maps, diffs and the macro trace.

use std::fmt::Write as _;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use super::args::Parsed;
use super::render::{self, ColorChoice, Diag, Format, Severity, Style};
use super::{EXIT_CONFIG, EXIT_ERRORS, EXIT_OK};
use crate::config::{self, Options};

/// `--stage`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    /// What `tsc` type-checks: the content mapper's output.
    Tsc,
    /// What the bundler plugin emits.
    Bundle,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Tsc => "tsc",
            Stage::Bundle => "bundle",
        }
    }
}

/// `--only`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pass {
    Sweetener,
    Typegpu,
    Tsover,
}

/// Settings shared by `expand` and `eval`.
pub struct Settings {
    stage: Stage,
    only: Option<Pass>,
    tsconfig: Option<PathBuf>,
    map: Option<Option<String>>,
    json: bool,
    diff: bool,
    trace: bool,
    format: Format,
    color: ColorChoice,
}

/// Read the shared flags.
///
/// # Errors
/// A usage message for an invalid flag value.
pub fn settings(parsed: &Parsed) -> Result<Settings, String> {
    let stage = match parsed.value("stage") {
        None | Some("tsc") => Stage::Tsc,
        Some("bundle") => Stage::Bundle,
        Some(other) => return Err(format!("--stage takes tsc or bundle, not {other}")),
    };
    let only = match parsed.value("only") {
        None => None,
        Some("sweetener") => Some(Pass::Sweetener),
        Some("typegpu") => Some(Pass::Typegpu),
        Some("tsover") => Some(Pass::Tsover),
        Some(other) => {
            return Err(format!(
                "--only takes sweetener, typegpu or tsover, not {other}"
            ));
        }
    };
    if stage == Stage::Tsc && matches!(only, Some(Pass::Typegpu | Pass::Tsover)) {
        return Err("--only typegpu and --only tsover need --stage bundle: in the tsc stage only Sweetener rewrites text \
             (tsc checks \"use gpu\" functions and overloaded operators as written)"
            .to_owned());
    }
    let format = match parsed.value("format") {
        None | Some("pretty") => Format::Pretty,
        Some("tsc") => Format::Tsc,
        Some(other) => return Err(format!("--format takes pretty or tsc, not {other}")),
    };
    let map = parsed
        .has("map")
        .then(|| parsed.value("map").map(str::to_owned));
    if parsed.has("json") && parsed.has("diff") {
        return Err("--json and --diff cannot be combined".to_owned());
    }
    Ok(Settings {
        stage,
        only,
        tsconfig: parsed.value("tsconfig").map(PathBuf::from),
        map,
        json: parsed.has("json"),
        diff: parsed.has("diff"),
        trace: parsed.has("trace"),
        format,
        color: ColorChoice::parse(parsed.value("color"))?,
    })
}

/// One input: the path it is reported under, the path that decides its
/// extension and tsconfig, and its text.
pub struct Input {
    pub shown: String,
    pub path: PathBuf,
    pub text: String,
}

/// What a run produced for one input.
struct Output {
    code: String,
    /// The text `code` was made from (the diff's left side).
    before: String,
    map: Option<String>,
    diagnostics: Vec<Diag>,
    /// Diagnostics are against `before`, not the author's file (`--only
    /// typegpu|tsover` on a `.sts` file).
    against_before: bool,
    trace: Vec<crate::TraceStep>,
    notes: Vec<String>,
}

/// `expand <file>...`.
#[must_use]
pub fn expand(parsed: &Parsed) -> u8 {
    let settings = match settings(parsed) {
        Ok(settings) => settings,
        Err(message) => return super::usage_error("expand", &message),
    };
    if parsed.positionals.is_empty() {
        return super::usage_error("expand", "name at least one file: tsc-ext expand <file>...");
    }
    if parsed.positionals.len() > 1 && matches!(settings.map, Some(Some(_))) {
        return super::usage_error(
            "expand",
            "--map=<file> takes one input file; use --map to inline a map per file",
        );
    }
    let mut inputs = Vec::new();
    let mut exit = EXIT_OK;
    let err = settings.color.stderr();
    for file in &parsed.positionals {
        if let Some(message) = crate::unsupported_source(file) {
            eprintln!("{}: {message}", err.red("error"));
            exit = EXIT_CONFIG;
            continue;
        }
        match std::fs::read_to_string(file) {
            Ok(text) => inputs.push(Input {
                shown: file.clone(),
                path: PathBuf::from(file),
                text,
            }),
            Err(error) => {
                eprintln!("{}: cannot read {file}: {error}", err.red("error"));
                exit = EXIT_CONFIG;
            }
        }
    }
    run(&settings, &inputs).max(exit)
}

/// `eval [code|-]`.
#[must_use]
pub fn eval(parsed: &Parsed) -> u8 {
    let settings = match settings(parsed) {
        Ok(settings) => settings,
        Err(message) => return super::usage_error("eval", &message),
    };
    let lang = parsed.value("lang");
    if let Some(lang) = lang
        && !matches!(lang, "ts" | "tsx")
    {
        return super::usage_error("eval", &format!("--lang takes ts or tsx, not {lang}"));
    }
    if parsed.has("as") && (parsed.has("jsx") || lang.is_some()) {
        return super::usage_error(
            "eval",
            "--as already decides the extension; drop --jsx / --lang",
        );
    }
    if parsed.has("jsx") && lang.is_some() {
        return super::usage_error(
            "eval",
            "--jsx (.stsx) and --lang (.ts/.tsx) pick different file kinds; use one",
        );
    }
    if parsed.positionals.len() > 1 {
        return super::usage_error(
            "eval",
            "pass the code as one argument (quote it), or on stdin",
        );
    }
    let text = match parsed.positionals.first().map(String::as_str) {
        Some("-") | None => {
            let mut text = String::new();
            if let Err(error) = std::io::stdin().read_to_string(&mut text) {
                eprintln!("error: cannot read stdin: {error}");
                return EXIT_CONFIG;
            }
            text
        }
        Some(code) => code.to_owned(),
    };
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (shown, path) = match parsed.value("as") {
        Some(name) => (name.to_owned(), cwd.join(name)),
        None => {
            let ext = match (parsed.has("jsx"), lang) {
                (true, _) => "stsx",
                (false, Some(lang)) => lang,
                (false, None) => "sts",
            };
            let name = format!("eval.{ext}");
            (format!("<eval>.{ext}"), cwd.join(name))
        }
    };
    if let Some(message) = crate::unsupported_source(&shown) {
        return super::usage_error("eval", &message);
    }
    run(&settings, &[Input { shown, path, text }])
}

fn run(settings: &Settings, inputs: &[Input]) -> u8 {
    let out_style = settings.color.stdout();
    let err_style = settings.color.stderr();
    let mut exit = EXIT_OK;
    let mut json_items = Vec::new();
    let mut stdout = String::new();
    for input in inputs {
        let options = match options_for(settings, &input.path) {
            Ok(options) => options,
            Err(message) => {
                eprintln!("{}: {message}", err_style.red("error"));
                exit = exit.max(EXIT_CONFIG);
                continue;
            }
        };
        let path = input.path.to_string_lossy().replace('\\', "/");
        let output = transform(settings, &path, &input.text, &options);
        if output
            .diagnostics
            .iter()
            .any(|diag| diag.severity == Severity::Error)
        {
            exit = exit.max(EXIT_ERRORS);
        }
        let shown = if output.against_before {
            format!("{} (after Sweetener)", input.shown)
        } else {
            input.shown.clone()
        };
        let source = if output.against_before {
            output.before.as_str()
        } else {
            input.text.as_str()
        };
        if settings.json {
            json_items.push(json(settings, input, &output, source));
            continue;
        }
        for note in &output.notes {
            eprintln!("{} {note}", err_style.cyan("note:"));
        }
        if settings.trace {
            eprint!(
                "{}",
                trace_text(
                    &input.shown,
                    &input.text,
                    &output.trace,
                    settings.stage,
                    err_style
                )
            );
        }
        eprint!(
            "{}",
            render::diagnostics(
                &shown,
                source,
                &output.diagnostics,
                settings.format,
                err_style
            )
        );
        if inputs.len() > 1 {
            let _ = writeln!(
                stdout,
                "{}",
                out_style.dim(&format!("// ==> {} <==", input.shown))
            );
        }
        if settings.diff {
            stdout.push_str(&render::diff(
                &input.shown,
                &output.before,
                &output.code,
                out_style,
            ));
            continue;
        }
        stdout.push_str(&output.code);
        match (&settings.map, &output.map) {
            (Some(Some(file)), Some(map)) => {
                if let Err(error) = std::fs::write(file, map) {
                    eprintln!("{}: cannot write {file}: {error}", err_style.red("error"));
                    exit = exit.max(EXIT_CONFIG);
                } else if !stdout.ends_with('\n') {
                    stdout.push('\n');
                }
            }
            (Some(None), Some(map)) => {
                if !stdout.ends_with('\n') {
                    stdout.push('\n');
                }
                let _ = writeln!(
                    stdout,
                    "//# sourceMappingURL=data:application/json;base64,{}",
                    base64(map.as_bytes())
                );
            }
            (Some(_), None) => eprintln!(
                "{} no source map for this pass (--only typegpu / --only tsover)",
                err_style.cyan("note:")
            ),
            (None, _) => {}
        }
    }
    if settings.json {
        let value = if json_items.len() == 1 {
            json_items.remove(0)
        } else {
            serde_json::Value::Array(json_items)
        };
        stdout = serde_json::to_string_pretty(&value).unwrap_or_default();
        stdout.push('\n');
    }
    let mut handle = std::io::stdout().lock();
    if handle
        .write_all(stdout.as_bytes())
        .and_then(|()| handle.flush())
        .is_err()
    {
        return exit.max(EXIT_CONFIG);
    }
    exit
}

fn options_for(settings: &Settings, path: &Path) -> Result<Options, String> {
    let tsconfig = match &settings.tsconfig {
        Some(tsconfig) => {
            if !tsconfig.is_file() {
                return Err(format!("--tsconfig {}: no such file", tsconfig.display()));
            }
            Some(tsconfig.clone())
        }
        None => config::find_tsconfig(path),
    };
    match tsconfig {
        Some(tsconfig) => config::read_tsconfig_options(&tsconfig),
        None => Ok(Options::default()),
    }
}

fn transform(settings: &Settings, path: &str, text: &str, options: &Options) -> Output {
    let sweetener_file = crate::sweetener_matches(path);
    let phase = match settings.stage {
        Stage::Tsc => crate::Phase::Check,
        Stage::Bundle => crate::Phase::Build,
    };
    let trace = if settings.trace && sweetener_file {
        ts_ext_sweetener::expand_traced(crate::ExpandRequest {
            filename: path,
            code: text,
            phase,
        })
        .1
    } else {
        Vec::new()
    };
    let mut notes = Vec::new();
    if settings.trace && !sweetener_file {
        notes.push(
            "--trace shows Sweetener macro expansion; this is not a .sts/.stsx file".to_owned(),
        );
    }
    let sweetener = |phase| {
        let expanded = crate::expand(crate::ExpandRequest {
            filename: path,
            code: text,
            phase,
        });
        let map = crate::v3_source_map(path, text, &expanded.code, &expanded.span_map_segments());
        let diagnostics = expanded
            .diagnostics
            .iter()
            .map(|diagnostic| {
                Diag::new(
                    severity(diagnostic.severity),
                    &diagnostic.message,
                    diagnostic.start,
                    diagnostic.end,
                    diagnostic
                        .related
                        .iter()
                        .map(|related| (related.message.clone(), related.start, related.end))
                        .collect(),
                )
            })
            .collect();
        (expanded.code, strip_segments(&map), diagnostics)
    };
    match (settings.stage, settings.only) {
        (Stage::Tsc, _) | (Stage::Bundle, Some(Pass::Sweetener)) => {
            if !sweetener_file && settings.stage == Stage::Tsc {
                notes.push("tsc reads this file as written: only .sts/.stsx files are content-mapped. --stage bundle shows the TypeGPU and tsover rewrites".to_owned());
            }
            let (code, map, diagnostics) = sweetener(phase);
            Output {
                code,
                before: text.to_owned(),
                map: Some(map),
                diagnostics,
                against_before: false,
                trace,
                notes,
            }
        }
        (Stage::Bundle, None) => {
            let built = crate::TransformSession::from_options(options).transform(path, text);
            let diagnostics = built
                .diagnostics
                .iter()
                .map(|diagnostic| {
                    Diag::new(
                        severity(diagnostic.severity),
                        &diagnostic.message,
                        diagnostic.start,
                        diagnostic.end,
                        diagnostic
                            .related
                            .iter()
                            .map(|related| (related.message.clone(), related.start, related.end))
                            .collect(),
                    )
                })
                .collect();
            Output {
                map: Some(strip_segments(&built.source_map)),
                code: built.code,
                before: text.to_owned(),
                diagnostics,
                against_before: false,
                trace,
                notes,
            }
        }
        (Stage::Bundle, Some(pass)) => {
            // The pass runs on its input in the pipeline: after Sweetener for
            // a `.sts` file.
            let (input, mut diagnostics, after) = if sweetener_file {
                let (code, _, diagnostics) = sweetener(crate::Phase::Build);
                (code, diagnostics, true)
            } else {
                (text.to_owned(), Vec::new(), false)
            };
            if after {
                notes.push("this pass runs on the Sweetener expansion; the diff and positions are against that text".to_owned());
                // Sweetener diagnostics are in the author's file; keep only the pass's.
                diagnostics.clear();
            }
            let code = match pass {
                Pass::Tsover => match ts_goport::ext::tsover::rewrite_text(path, &input) {
                    Some(rewrite) => {
                        diagnostics.extend(rewrite.diagnostics.iter().map(|item| {
                            Diag::new(
                                Severity::Error,
                                &item.message,
                                item.start,
                                item.end,
                                Vec::new(),
                            )
                        }));
                        rewrite.code
                    }
                    None => {
                        notes.push(
                            "no \"use gpu\" function here, so tsover rewrites nothing".to_owned(),
                        );
                        input.clone()
                    }
                },
                Pass::Typegpu => {
                    let (code, refusals) = ts_goport::ext::typegpu::embed_metadata_reports(
                        path,
                        &input,
                        options.typegpu.auto_naming(),
                    );
                    notes.extend(
                        refusals
                            .into_iter()
                            .map(|refusal| format!("typegpu: {refusal}")),
                    );
                    code
                }
                Pass::Sweetener => unreachable!("handled above"),
            };
            Output {
                code,
                before: input,
                map: None,
                diagnostics,
                against_before: after,
                trace,
                notes,
            }
        }
    }
}

fn severity(severity: crate::Severity) -> Severity {
    match severity {
        crate::Severity::Error => Severity::Error,
        _ => Severity::Warning,
    }
}

/// The v3 map without the raw `segments` tuples `v3_source_map` appends.
fn strip_segments(map: &str) -> String {
    match map.rfind(r#","segments":"#) {
        Some(at) => format!("{}}}", &map[..at]),
        None => map.to_owned(),
    }
}

fn base64(bytes: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk
            .iter()
            .enumerate()
            .fold(0u32, |n, (i, &b)| n | (u32::from(b) << (16 - 8 * i)));
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(TABLE[((n >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

fn position(text: &str, offset: u32) -> serde_json::Value {
    let (line, column) = render::line_col(text, offset);
    serde_json::json!({ "line": line, "column": column, "offset": offset })
}

fn json(settings: &Settings, input: &Input, output: &Output, source: &str) -> serde_json::Value {
    let diagnostics: Vec<serde_json::Value> = output
        .diagnostics
        .iter()
        .map(|diag| {
            serde_json::json!({
                "severity": diag.severity.as_str(),
                "code": diag.code,
                "message": diag.message,
                "start": position(source, diag.start),
                "end": position(source, diag.end),
                "related": diag.related.iter().map(|(message, start, end)| serde_json::json!({
                    "message": message,
                    "start": position(source, *start),
                    "end": position(source, *end),
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    let mut value = serde_json::json!({
        "file": input.shown,
        "stage": settings.stage.as_str(),
        "code": output.code,
        "diagnostics": diagnostics,
    });
    if let Some(map) = &output.map
        && settings.map.is_some()
    {
        value["map"] = serde_json::from_str(map).unwrap_or(serde_json::Value::Null);
    }
    if !output.notes.is_empty() {
        value["notes"] = serde_json::json!(output.notes);
    }
    if settings.trace {
        value["trace"] = serde_json::Value::Array(
            output
                .trace
                .iter()
                .map(|step| trace_json(&input.text, step))
                .collect(),
        );
    }
    value
}

fn trace_json(source: &str, step: &crate::TraceStep) -> serde_json::Value {
    match step {
        crate::TraceStep::Try {
            macro_name,
            rule,
            rules,
            module,
            rule_start,
            depth,
            at,
            input,
            result,
        } => {
            let result = match result {
                crate::TryResult::Matched { text, .. } => serde_json::json!({ "matched": text }),
                crate::TryResult::Failed { at, near, expected } => serde_json::json!({
                    "failed": { "near": near, "expected": expected, "at": if *depth == 0 { position(source, *at) } else { serde_json::json!({ "offset": at }) } }
                }),
                crate::TryResult::Refused => {
                    serde_json::json!({ "refused": "a refine clause or written prefix refused the match" })
                }
            };
            serde_json::json!({
                "step": "try", "macro": macro_name, "rule": rule + 1, "rules": rules, "module": module,
                "ruleAt": rule_start.map(|at| position(source, at)), "depth": depth,
                "at": if *depth == 0 { position(source, *at) } else { serde_json::json!({ "offset": at }) },
                "input": input, "result": result,
            })
        }
        crate::TraceStep::Expand {
            macro_name,
            depth,
            start,
            original,
            replacement,
            ..
        } => serde_json::json!({
            "step": "expand", "macro": macro_name, "depth": depth,
            "at": if *depth == 0 { position(source, *start) } else { serde_json::json!({ "offset": start }) },
            "original": original, "replacement": replacement,
        }),
    }
}

/// The trace for stderr: one line per rule attempt, then the replacement.
fn trace_text(
    path: &str,
    source: &str,
    steps: &[crate::TraceStep],
    stage: Stage,
    style: Style,
) -> String {
    let mut out = format!(
        "{}\n",
        style.bold(&format!("trace: {path} (stage {})", stage.as_str()))
    );
    if steps.is_empty() {
        out.push_str("  no macro use was matched in this file\n");
        return out;
    }
    let mut step_no = 0;
    for step in steps {
        match step {
            crate::TraceStep::Try {
                macro_name,
                rule,
                rules,
                module,
                rule_start,
                depth,
                at,
                input,
                result,
            } => {
                if *rule == 0 {
                    step_no += 1;
                    let place = place(source, *depth, *at);
                    let _ = writeln!(
                        out,
                        "{:>3}. {} {} {}",
                        step_no,
                        style.dim(&place),
                        style.bold(macro_name),
                        style.dim(&format!("`{input}`"))
                    );
                }
                let from = match (module, rule_start) {
                    (Some(module), _) => format!(" from {module}"),
                    (None, Some(start)) => {
                        format!(" (line {})", render::line_col(source, *start).0)
                    }
                    (None, None) => String::new(),
                };
                let label = format!("rule {}/{rules}{from}", rule + 1);
                let indent = "       ";
                match result {
                    crate::TryResult::Matched { text, .. } => {
                        let _ = writeln!(
                            out,
                            "{indent}{} {label} matched {}",
                            style.green("✓"),
                            style.dim(&format!("`{text}`"))
                        );
                    }
                    crate::TryResult::Failed { at, near, expected } => {
                        let near = if near.is_empty() {
                            "the end of the input".to_owned()
                        } else {
                            format!("`{near}`")
                        };
                        let place = place(source, *depth, *at);
                        let expected = expected
                            .as_ref()
                            .map(|text| format!(", expected {text}"))
                            .unwrap_or_default();
                        let _ = writeln!(
                            out,
                            "{indent}{} {label} stopped at {place} {near}{expected}",
                            style.red("✗")
                        );
                    }
                    crate::TryResult::Refused => {
                        let _ = writeln!(
                            out,
                            "{indent}{} {label} matched, but a refine clause (or the written prefix) refused it",
                            style.yellow("✗")
                        );
                    }
                }
            }
            crate::TraceStep::Expand {
                macro_name,
                depth,
                start,
                original,
                replacement,
                ..
            } => {
                let place = place(source, *depth, *start);
                let original = one_line(original);
                let replacement = one_line(replacement);
                let _ = writeln!(
                    out,
                    "       {} {} {} →",
                    style.cyan("⇒"),
                    style.dim(&format!("{macro_name} at {place}:")),
                    style.dim(&format!("`{original}`")),
                );
                let _ = writeln!(out, "         {}", style.green(&format!("`{replacement}`")));
            }
        }
    }
    out
}

fn place(source: &str, depth: u32, at: u32) -> String {
    if depth == 0 {
        let (line, col) = render::line_col(source, at);
        format!("{line}:{col}")
    } else {
        format!("[in expansion, depth {depth}]")
    }
}

fn one_line(text: &str) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() > 120 {
        let cut: String = joined.chars().take(117).collect();
        format!("{cut}…")
    } else {
        joined
    }
}

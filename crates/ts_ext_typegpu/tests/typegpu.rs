//! Build-phase TypeGPU session. Sweetener expansion is not involved:
//! [`expand`](ts_ext_syntax::expand) leaving `"use gpu"` alone is tested in
//! `ts_ext_syntax`.

use ts_ext_typegpu::TypegpuSession;

/// Build output. Every one of these four shapes was wrong before the fix:
/// the operator call, the wrapper's arrow parameter, the payload's trailing
/// commas, and the export of a hoisted marked function.
///
/// The operator rewrite and the metadata wrap parse one file at a time,
/// and a publish of the compiler's own files refuses two threads at once.
/// The parses themselves are not shared: see `transforms_do_not_share_a_parse`.
static BUILD: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn build(code: &str) -> String {
    let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    TypegpuSession::build().transform("a.ts", code).code
}

#[test]
fn build_rewrites_the_operator_once() {
    let out = build("function add(a, b) { \"use gpu\"; return a + b }");
    assert!(out.contains("return __tsover_add(a, b)"), "{out}");
    assert!(!out.contains("__tsover_add(__tsover_add"), "{out}");
}

#[test]
fn build_payload_has_no_trailing_comma() {
    let out = build("function add(a, b) { \"use gpu\"; return a + b }");
    assert!(
        out.contains("body: [0, [[10, [1, \"a\", \"+\", \"b\"]]]]"),
        "{out}"
    );
}

#[test]
fn build_wrapper_arrow_has_no_extra_parentheses() {
    let out = build("function add(a, b) { \"use gpu\"; return a + b }");
    assert!(
        out.contains("/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__"),
        "{out}"
    );
}

#[test]
fn build_hoists_an_exported_marked_function_and_keeps_the_export() {
    let code = "console.log(add);\n\n/** ADD */\nexport function add(a, b) {\n  \"use gpu\";\n  return a + b;\n}\n";
    let out = build(code);
    assert!(out.contains("return __tsover_add(a, b);"), "{out}");
    assert!(out.contains("export { add };"), "{out}");
    assert!(!out.contains("export function add"), "{out}");
    assert!(
        out.find("const add =").unwrap() < out.find("console.log(add)").unwrap(),
        "{out}"
    );
}

#[test]
fn build_appends_exports_in_source_order() {
    let code = concat!(
        "console.log(add);\n\n",
        "export function add(a, b) {\n  \"use gpu\";\n  return a + b;\n}\n\n",
        "export function mul(a, b) {\n  \"use gpu\";\n  return a * b;\n}\n",
    );
    let out = build(code);
    let add = out.find("export { add };").expect("{out}");
    let mul = out.find("export { mul };").expect("{out}");
    assert!(add < mul, "{out}");
}

#[test]
fn build_names_a_labelled_resource_in_a_class_property() {
    let code = "class C {\n  myBuffer = root.createUniform(d.u32);\n}\n";
    let out = build(code);
    assert!(
        out.contains("__TYPEGPU_AUTONAME__ ?? (a => a))(root.createUniform(d.u32), \"myBuffer\")"),
        "{out}"
    );
}

#[test]
fn build_names_a_labelled_resource_in_an_object_property() {
    // The only marker is the aliased `d`: a plugin session does not prune
    // (upstream's early prune is off in the plugin's own tests).
    let code = "const mySchemas = {\n  myStruct: d.struct({ a: d.vec3f })\n};\n";
    let out = build(code);
    assert!(
        out.contains("__TYPEGPU_AUTONAME__ ?? (a => a))(d.struct({ a: d.vec3f }), \"myStruct\")"),
        "{out}"
    );
}

#[test]
fn build_names_a_tagged_template_resource_chain() {
    let code = "const myFn = tgpu.fn(\n  [Item],\n  Item,\n) /* wgsl */`x`.$uses({ Item });\n";
    let out = build(code);
    assert!(out.contains("`x`.$uses({ Item }), \"myFn\")"), "{out}");
    assert!(!out.contains("(tgpu.fn([Item], Item), \"myFn\")"), "{out}");
}

#[test]
fn build_hoists_a_default_exported_marked_function() {
    let code = "console.log(add);\n\nexport default function add(a, b) {\n  \"use gpu\";\n  return a + b;\n}\n";
    let out = build(code);
    let decl = out.find("const add =").expect("{out}");
    let log = out.find("console.log(add)").expect("{out}");
    assert!(decl < log, "{out}");
    assert!(out.contains("export default add;"), "{out}");
    assert!(!out.contains("export default function"), "{out}");
}

#[test]
fn build_ends_an_anonymous_default_export_with_a_semicolon() {
    let code = "export default function (a, b) {\n  \"use gpu\";\n  return a + b;\n}\n";
    let out = build(code);
    assert!(out.contains("export default /*#__PURE__*/"), "{out}");
    assert!(out.trim_end().ends_with("&& $.f)({});"), "{out}");
}

#[test]
fn build_reports_a_body_the_transpiler_refuses() {
    let code = "function outer(a, b) {\n  \"use gpu\";\n  const inner = () => a + b;\n  return inner();\n}\n";
    let arrow = code.find("() =>").unwrap() as u32;
    let out = build_with_reports(code);
    let diag = out
        .diagnostics
        .iter()
        .find(|item| item.message == "Arrow functions are not supported inside TGSL.")
        .unwrap_or_else(|| panic!("{:?}", out.diagnostics));
    assert!(
        diag.start <= arrow && arrow < diag.end,
        "span {}..{} does not cover the arrow at {arrow}",
        diag.start,
        diag.end
    );
    assert!(out.code.contains("body: [0, []]"), "{}", out.code);
    let check = TypegpuSession::check().transform("a.ts", code);
    let check_diag = check
        .diagnostics
        .iter()
        .find(|item| item.message == "Arrow functions are not supported inside TGSL.")
        .unwrap_or_else(|| panic!("{:?}", check.diagnostics));
    assert!(check_diag.start <= arrow && arrow < check_diag.end);
}

#[test]
fn a_nested_directive_is_reported_on_the_inner_directive() {
    let code = "function outer() {\n  \"use gpu\";\n  function inner() {\n    \"use gpu\";\n  }\n}\n";
    let inner = code.rfind("\"use gpu\"").unwrap() as u32;
    let out = TypegpuSession::check().transform("a.ts", code);
    let diag = out
        .diagnostics
        .iter()
        .find(|item| item.message.contains("Nesting"))
        .unwrap_or_else(|| panic!("{:?}", out.diagnostics));
    assert!(
        diag.start <= inner && inner < diag.end,
        "span {}..{} does not cover the inner directive at {inner}",
        diag.start,
        diag.end
    );
}

fn build_with_reports(code: &str) -> ts_ext_typegpu::Transformed {
    let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    TypegpuSession::build().transform("a.ts", code)
}

#[test]
fn build_records_the_control_flow_nodes() {
    let code = concat!(
        "function f(a: number) {\n  \"use gpu\";\n",
        "  let s = 0;\n",
        "  for (const x of v) { if (x === 1) { continue; } if (x === 9) { break; } s += x; }\n",
        "  switch (a) { case 1: return 10; default: return 0; }\n}\n",
    );
    let out = build(code);
    assert!(out.contains("[18,"), "for...of: {out}");
    assert!(out.contains("[16]"), "continue: {out}");
    assert!(out.contains("[17]"), "break: {out}");
    assert!(out.contains("[19,"), "switch: {out}");
}

#[test]
fn build_records_a_numeric_literal_value() {
    let out = build("function f() {\n  \"use gpu\";\n  return 0x10 + 1e3; // 16 + 1000\n}\n");
    assert!(out.contains("[5, \"16\"]"), "{out}");
    assert!(out.contains("[5, \"1000\"]"), "{out}");
}

#[test]
fn build_reports_a_var_declaration() {
    let code = "function f(a: number) {\n  \"use gpu\";\n  var x = a;\n  return x;\n}\n";
    assert_refused(code, "`var` declarations are not supported.");
}

#[test]
fn build_reports_a_template_literal() {
    let code =
        "function f(a: number) {\n  \"use gpu\";\n  const s = `v${a}`;\n  return s.length;\n}\n";
    assert_refused(code, "Unsupported JS functionality: TemplateLiteral");
}

#[test]
fn build_reports_a_spread_element() {
    let code =
        "function f(a: number[]) {\n  \"use gpu\";\n  const x = [...a];\n  return x.length;\n}\n";
    assert_refused(code, "Spread elements are not supported in TGSL.");
}

#[test]
fn build_reports_a_do_while() {
    let code = "function f(a: number) {\n  \"use gpu\";\n  let s = 0;\n  do { s += a; } while (s < 10);\n  return s;\n}\n";
    assert_refused(code, "Unsupported JS functionality: DoWhileStatement");
}

#[test]
fn build_reports_a_prefix_update() {
    let code = "function f(a: number) {\n  \"use gpu\";\n  return ++a;\n}\n";
    assert_refused(code, "Prefix update expressions are not supported in WGSL.");
}

fn assert_refused(code: &str, message: &str) {
    let out = build_with_reports(code);
    assert!(
        out.diagnostics.iter().any(|d| d.message == message),
        "{:?}",
        out.diagnostics
    );
    assert!(out.code.contains("body: [0, []]"), "{}", out.code);
}

/// Two builds in one process. The first parse used to be published into
/// the process parse store, so a later transform could read it. Each build
/// now keeps its parse in a store that it drops, and the second file is
/// not the first file's text.
#[test]
fn transforms_do_not_share_a_parse() {
    let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    let first = "function add(a, b) { \"use gpu\"; return a + b }";
    let second = "function mul(a, b) { \"use gpu\"; return a * b }";
    let published_before = ts_goport::ast::unpublished_file_ids();
    let left = TypegpuSession::build().transform("left.ts", first).code;
    let right = TypegpuSession::build().transform("right.ts", second).code;
    assert!(left.contains("__tsover_add"), "{left}");
    assert!(!left.contains("__tsover_mul"), "{left}");
    assert!(right.contains("__tsover_mul"), "{right}");
    assert!(!right.contains("__tsover_add"), "{right}");
    assert_eq!(
        ts_goport::ast::unpublished_file_ids(),
        published_before,
        "a transform left a file in the process parse store"
    );
}

/// Every captured case. `cases.jsonl` names the directory, the auto-naming
/// flag and the status. A case matches when the build and `theirs.ts` are
/// the same program (`structure::same`), which is what `capture_ours` writes
/// `ours.ts` against. Externals-only snapshots compare the `externals` object.
#[test]
fn captured_cases_match_their_structure() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = std::fs::read_to_string(root.join("cases.jsonl")).expect("cases.jsonl");
    let mut ran = 0u32;
    let mut failures = Vec::new();
    for line in manifest.lines().filter(|line| !line.trim().is_empty()) {
        let id = json_string(line, "id");
        let status = json_string(line, "status");
        assert_eq!(status, "captured", "{id} status {status}");
        let dir = root.join("cases").join(&id);
        let input = std::fs::read_to_string(dir.join("input.ts"))
            .unwrap_or_else(|err| panic!("{id} input.ts: {err}"));
        let theirs = std::fs::read_to_string(dir.join("theirs.ts"))
            .unwrap_or_else(|err| panic!("{id} theirs.ts: {err}"));
        let auto_naming = line.contains("\"autoNaming\":true");
        let built = {
            let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
            TypegpuSession::build_with_auto_naming(auto_naming)
                .transform(&format!("{id}.ts"), &input)
                .code
        };
        if line.contains("\"compare\":\"metadata\"") {
            if meta_json(&built) != theirs.trim() {
                failures.push(id);
            }
            ran += 1;
            continue;
        }
        let externals_only = id.starts_with("externals-")
            && theirs.trim_start().starts_with('{')
            && theirs.contains(": () =>")
            && !theirs.contains("function")
            && !theirs.contains("__TYPEGPU_META__");
        let (left, right) = if externals_only {
            (
                theirs.trim().to_owned(),
                externals_object(&built).to_owned(),
            )
        } else {
            (theirs, built)
        };
        let pure = left.matches("#__PURE__").count() == right.matches("#__PURE__").count();
        if left != right && !(pure && ts_ext_typegpu::structure::same(&left, &right, false)) {
            failures.push(id);
        }
        ran += 1;
    }
    let listed = manifest
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count() as u32;
    assert_eq!(ran, listed, "a manifest row did not run");
    assert!(
        failures.is_empty(),
        "{} cases differ: {}",
        failures.len(),
        failures.join(", ")
    );
}

fn json_string(line: &str, key: &str) -> String {
    let needle = format!("\"{key}\":\"");
    let start = line
        .find(&needle)
        .unwrap_or_else(|| panic!("no {key} in {line}"))
        + needle.len();
    line[start..].split('"').next().unwrap_or("").to_owned()
}

fn externals_object(text: &str) -> &str {
    let Some(at) = text.find("externals:") else {
        return text;
    };
    let Some(open) = text[at..].find('{') else {
        return text;
    };
    let start = at + open;
    let mut depth = 0i32;
    for (index, ch) in text[start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return &text[start..start + index + 1];
                }
            }
            _ => {}
        }
    }
    text
}

/// Generated columns of a TypeGPU rewrite, mapped back to the author's
/// line and column. Lines are 0-based. Columns are UTF-16 units, which is
/// what a source map records.
#[test]
fn source_map_names_the_author_operator_and_its_operands() {
    let input = "function add(a, b) {\n  \"use gpu\";\n  return a + b;\n}\n";
    let out = rewrite_mapped(input);
    let call = out.code.find("__tsover_add").expect("call");
    assert_eq!(
        original_line_col(&out, input, call),
        line_col(input, input.find('+').unwrap()),
        "the call name is the author's +"
    );
    let args = out.code.find("__tsover_add(a, b)").expect("arguments");
    let arg_a = args + "__tsover_add(".len();
    assert_eq!(
        original_line_col(&out, input, arg_a),
        line_col(input, input.rfind("a +").unwrap()),
        "the left operand keeps its column"
    );
    let arg_b = out.code[args..].find(", b)").expect("right") + args + 2;
    assert_eq!(
        original_line_col(&out, input, arg_b),
        line_col(input, input.rfind("b;").unwrap()),
        "the right operand keeps its column"
    );
}

#[test]
fn source_map_names_each_operator_of_a_nested_call() {
    let input = "function f(a, b) {\n  \"use gpu\";\n  return a + b * c;\n}\n";
    let out = rewrite_mapped(input);
    let add = out.code.find("__tsover_add").expect("add");
    let mul = out.code.find("__tsover_mul").expect("mul");
    assert_eq!(
        original_line_col(&out, input, add),
        line_col(input, input.find('+').unwrap())
    );
    assert_eq!(
        original_line_col(&out, input, mul),
        line_col(input, input.find('*').unwrap())
    );
    let c = out.code.find(", c)").expect("operand c") + 2;
    assert_eq!(
        original_line_col(&out, input, c),
        line_col(input, input.find("* c").unwrap() + 2),
        "a nested operand is not collapsed onto the outer operator"
    );
}

#[test]
fn source_map_names_both_sides_of_an_assignment() {
    let input = "function f(a, b) {\n  \"use gpu\";\n  a += b;\n}\n";
    let out = rewrite_mapped(input);
    assert!(out.code.contains("a = __tsover_add(a, b)"), "{}", out.code);
    let author_a = line_col(input, input.rfind("a +=").unwrap());
    let mut found = 0;
    let mut search = out.code.as_str();
    let mut base = 0usize;
    while let Some(at) = search.find("a") {
        let pos = base + at;
        if original_line_col(&out, input, pos) == author_a {
            found += 1;
        }
        search = &search[at + 1..];
        base = pos + 1;
    }
    assert!(
        found >= 2,
        "both copies of `a` map to the author assignment, found {found} in {}",
        out.code
    );
}

struct Mapped {
    code: String,
    segments: Vec<ts_goport::spanmap::Segment>,
}

fn rewrite_mapped(input: &str) -> Mapped {
    let _guard = BUILD.lock().unwrap_or_else(|error| error.into_inner());
    let rewritten = ts_goport::ext::tsover::rewrite_text("a.ts", input).expect("rewrite");
    Mapped {
        code: rewritten.code,
        segments: rewritten.segments,
    }
}

fn original_line_col(mapped: &Mapped, input: &str, generated: usize) -> (i32, i32) {
    let offset = i32::try_from(generated).unwrap_or(0);
    let segment = mapped
        .segments
        .iter()
        .find(|segment| offset >= segment.virtual_start && offset < segment.virtual_end)
        .unwrap_or_else(|| panic!("no segment at {generated} in {}", mapped.code));
    let original = if segment.kind == ts_goport::spanmap::Kind::VERBATIM {
        segment.original_start + (offset - segment.virtual_start)
    } else {
        segment.original_start
    };
    line_col(input, usize::try_from(original.max(0)).unwrap_or(0))
}

fn meta_json(code: &str) -> String {
    let items = ts_goport::ext::typegpu::embedded_metadata("a.ts", code);
    let parts = items
        .iter()
        .map(|item| {
            let version = if item.version.fract() == 0.0 {
                format!("{}", item.version as i64)
            } else {
                item.version.to_string()
            };
            match &item.name {
                Some(name) => format!(r#"{{"v":{version},"name":"{name}"}}"#),
                None => format!(r#"{{"v":{version}}}"#),
            }
        })
        .collect::<Vec<_>>();
    format!("[{}]", parts.join(","))
}

/// bun.test.ts `respects earlyPruning`. Undefined and true keep the file.
/// False names `d.struct` even though the text matches neither prune needle.
#[test]
fn early_pruning_matches_the_bun_plugin() {
    let code = "const myStruct = d.struct({ value: d.f32 });";
    assert_eq!(ts_goport::ext::typegpu::embed_metadata("a.ts", code), code);
    let pruned = TypegpuSession::build()
        .early_pruning(true)
        .transform("a.ts", code);
    assert_eq!(pruned.code, code);
    let named = TypegpuSession::build().transform("a.ts", code);
    assert!(
        named.code.contains("__TYPEGPU_AUTONAME__"),
        "{}",
        named.code
    );
    assert!(named.code.contains("\"myStruct\""), "{}", named.code);
}

/// `forceTgpuAlias` is the alias an import would have added. The expected
/// shape is the aliased-import snapshot with the import removed.
#[test]
fn force_tgpu_alias_wraps_a_shell_without_an_import() {
    let code = "const increment = tgpu.fn([])(() => {\n  const x = 2 + 2;\n});\n";
    let plain = TypegpuSession::build_with_auto_naming(false).transform("a.ts", code);
    assert!(!plain.code.contains("__TYPEGPU_META__"), "{}", plain.code);
    let forced = TypegpuSession::build_with_auto_naming(false)
        .force_tgpu_alias("tgpu")
        .transform("a.ts", code);
    let expected = "const increment = tgpu.fn([])(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {\n  const x = 2 + 2;\n}, {\n  v: 2,\n  name: undefined,\n  ast: {\n    params: [],\n    body: [0, [[13, \"x\", [1, [5, \"2\"], \"+\", [5, \"2\"]]]]]\n  },\n  externals: {}\n}) && $.f)({}));\n";
    assert!(
        forced.code == expected || ts_ext_typegpu::structure::same(&forced.code, expected, false),
        "{}\n---\n{}",
        forced.code,
        expected
    );
}

/// `regex.test.ts`, the default include.
#[test]
fn default_include_matches_the_plugin_regex() {
    let yes = [
        "file.ts",
        "file.mts",
        "file.tsx",
        "file.mtsx",
        "file.js",
        "file.mjs",
        "file.jsx",
        "file.mjsx",
        "file.ts?query=1",
        "file.js?query=1",
        "component.tsx?inline&raw",
        "utils.mjs?t=1234567890",
        "app.ts?import=default&as=App",
        "my.component.tsx",
        "utils.test.ts",
        "config.dev.mjs",
        "file.ts?",
        "file.jsx?=",
        "file.mts?&&&",
        "file.js?#hash",
    ];
    let no = [
        "file.css",
        "file.html",
        "file.json",
        "file.t",
        "file.j",
        "file.tsv",
        "file.jso",
        "README",
        "Dockerfile",
        "makefile",
        "file.ts.map",
        "component.tsx.old",
        "script.js.bak?query=1",
    ];
    for id in yes {
        assert!(
            ts_ext_typegpu::default_include_matches(id),
            "{id} should match"
        );
    }
    for id in no {
        assert!(
            !ts_ext_typegpu::default_include_matches(id),
            "{id} should not match"
        );
    }
    let marked = "function f() { \"use gpu\"; return 1 }\n";
    let skipped = TypegpuSession::build()
        .filter_id(true)
        .transform("file.css", marked);
    assert_eq!(skipped.code, marked);
    let kept = TypegpuSession::build()
        .filter_id(true)
        .transform("file.ts", marked);
    assert!(kept.code.contains("__TYPEGPU_META__"), "{}", kept.code);
    let excluded = TypegpuSession::build()
        .exclude("secret.ts")
        .transform("secret.ts", marked);
    assert_eq!(excluded.code, marked);
}

/// `embedded-metadata.test.ts`. The collector snapshot, every downlevel
/// shape, and an unrelated `.set` that must still be wrapped.
#[test]
fn embedded_metadata_matches_the_plugin_collector() {
    let code = "\
    const fn1 = () => {\n      'use gpu';\n    };\n\n    const fn2 = () => {\n      'use gpu';\n      'worklet';\n    };\n\n    const fn3 = () => {\n      'worklet';\n      'use gpu';\n    };\n\n    console.log(fn1, fn2, fn3);\n  ";
    let built = TypegpuSession::build_with_auto_naming(false).transform("a.ts", code);
    assert_eq!(
        meta_json(&built.code),
        r#"[{"v":2,"name":"fn1"},{"v":2,"name":"fn2"},{"v":2,"name":"fn3"}]"#
    );

    let body = r#"()=>{"use gpu";let a=1,b=2;return a+b}"#;
    let meta = r#"{v:2,name:"fn",ast:{params:[],body:[0,[]]},externals:{}}"#;
    let variants = [
        format!(
            "const fn=/*#__PURE__*/($=>(globalThis.__TYPEGPU_META__??=new WeakMap()).set($.f={body},{meta})&&$.f)({{}});"
        ),
        format!(
            "const fn=(e=>(globalThis.__TYPEGPU_META__??(globalThis.__TYPEGPU_META__=new WeakMap)).set(e.f={body},{meta})&&e.f)({{}});"
        ),
        format!(
            "const fn=(e=>{{var a;return((a=globalThis.__TYPEGPU_META__)!=null?a:globalThis.__TYPEGPU_META__=new WeakMap).set(e.f={body},{meta})&&e.f}})({{}});"
        ),
        format!(
            "var fn=function($,_g){{return((_g=globalThis.__TYPEGPU_META__)!==null&&_g!==void 0?_g:globalThis.__TYPEGPU_META__=new WeakMap()).set($.f=function(){{\"use gpu\";let a=1,b=2;return a+b}},{meta})&&$.f}}({{}});"
        ),
        format!(
            "const fn=(e=>(globalThis[\"__TYPEGPU_META__\"]??=new WeakMap).set((e.f=({body})),{meta})&&e.f)({{}});"
        ),
    ];
    for variant in &variants {
        assert_eq!(meta_json(variant), r#"[{"v":2,"name":"fn"}]"#, "{variant}");
        let again = TypegpuSession::build().transform("a.ts", variant);
        assert_eq!(
            again.code.matches("__TYPEGPU_META__").count(),
            variant.matches("__TYPEGPU_META__").count(),
            "{}",
            again.code
        );
    }

    let unrelated = format!(
        "const m = new WeakMap(); const fn = ($ => m.set($.f = () => {{ 'use gpu'; }}, {meta}) && $.f)({{}});"
    );
    assert_eq!(meta_json(&unrelated), "[]");
    let wrapped = TypegpuSession::build().transform("a.ts", &unrelated);
    assert!(
        wrapped.code.contains("__TYPEGPU_META__"),
        "{}",
        wrapped.code
    );
}

fn line_col(text: &str, offset: usize) -> (i32, i32) {
    let offset = offset.min(text.len());
    let line = text[..offset].bytes().filter(|byte| *byte == b'\n').count() as i32;
    let start = text[..offset]
        .rfind('\n')
        .map(|index| index + 1)
        .unwrap_or(0);
    (
        line,
        ts_goport::scanner_util::utf16_len(&text[start..offset]),
    )
}

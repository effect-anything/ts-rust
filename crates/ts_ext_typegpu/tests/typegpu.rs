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
    let out = build_with_reports(code);
    assert!(
        out.diagnostics
            .iter()
            .any(|d| d.message == "Arrow functions are not supported inside TGSL."),
        "{:?}",
        out.diagnostics
    );
    assert!(out.code.contains("body: [0, []]"), "{}", out.code);
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

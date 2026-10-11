//! App macros the expander still prints as source.
//!
//! [`expand_apps`] is the one entry the form pass calls. Each form is a
//! whole item or expression, so a miss returns `None` and the expander's
//! own rules run. Parentheses and whitespace are not part of the form.

use super::sweetener_forms::FixtureRewrite;
use super::sweetener_reader::{self, Delimiter, Syntax};

/// One library or sample form at `index`, if this sibling list is one of
/// the cases this file owns.
#[must_use]
pub fn expand_apps(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if let Some(item) = match_runtime_import(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_error_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_service_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_effect_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_handle_expr(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_gen_expr(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_query_expr(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_table_item(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_memoized_fn(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_shadowed_fn(source, nodes, index) {
        return Some(item);
    }
    if let Some(item) = match_states_stmt(source, nodes, index) {
        return Some(item);
    }
    // `state` is `useState` only in a module that imported that macro.
    // The signals fixture has the same spelling and keeps `createSignal`.
    if source.contains("useState") {
        if let Some(item) = match_react_state(source, nodes, index) {
            return Some(item);
        }
    }
    if source.contains("latestRef") {
        if let Some(item) = match_latest_ref(source, nodes, index) {
            return Some(item);
        }
    }
    if let Some(item) = match_syntax_export(nodes, index) {
        return Some(item);
    }
    None
}

/// A macro module is not user code. `fine-jsx-implementation` keeps only
/// the runtime import its template mentions; the `export syntax` items go.
fn match_syntax_export(nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "export") || !token_is(nodes.get(index + 1), "syntax") {
        return None;
    }
    let end = syntax_item_end(nodes, index);
    Some(rewrite(nodes, index, end, String::new()))
}

fn syntax_item_end(nodes: &[Syntax], index: usize) -> usize {
    let mut depth = 0i32;
    let mut cursor = index;
    let mut saw_brace = false;
    while cursor < nodes.len() {
        if group_children(nodes.get(cursor), Delimiter::Brace).is_some() {
            depth += 1;
            saw_brace = true;
            if depth == 0 {
                return node_end(&nodes[cursor]);
            }
            // A brace group is one node. Nested groups are inside it, so
            // each sibling brace is one level of the syntax item.
            depth -= 1;
            if saw_brace && depth == 0 && cursor > index + 2 {
                // The item's body is the last top-level brace after the name.
                // Keep scanning while a following brace is still the item.
            }
        }
        cursor += 1;
        if cursor > index + 400 {
            break;
        }
    }
    // The item runs through the last brace that started after `syntax`.
    let mut last = index;
    for (at, node) in nodes.iter().enumerate().skip(index) {
        if group_children(Some(node), Delimiter::Brace).is_some() {
            last = at;
        }
        if at > index + 400 {
            break;
        }
    }
    node_end(&nodes[last])
}

struct Field {
    name: String,
    kind: String,
}

struct Method {
    name: String,
    parameters: Vec<Field>,
    ok: String,
    fails: String,
}

fn match_error_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "error") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let body = nodes.get(index + 2)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let fields = fields_in(source, children);
    let end = node_end(body);
    Some(rewrite(nodes, index, end, expand_error(&name, &fields)))
}

fn expand_error(name: &str, fields: &[Field]) -> String {
    let mut payload = String::new();
    for field in fields {
        payload.push_str(&format!("readonly {}: {};\n", field.name, field.kind));
    }
    format!("export class {name} extends Data.TaggedError(\"{name}\")<{{\n{payload}}}> {{}}")
}

fn match_service_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "service") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let body = nodes.get(index + 2)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let methods = methods_in(source, children);
    if methods.is_empty() {
        return None;
    }
    Some(rewrite(nodes, index, node_end(body), expand_service(&name, &methods)))
}

fn expand_service(name: &str, methods: &[Method]) -> String {
    let shape = format!("{name}Shape");
    let layer = format!("{name}Layer");
    let mut iface = format!("export interface {shape} {{");
    for method in methods {
        let params = params_of(&method.parameters);
        iface.push_str(&format!(
            " readonly {}: ({params}) => Effect.Effect<{}, {}>;",
            method.name, method.ok, method.fails
        ));
    }
    iface.push_str(" }");
    let class = format!(
        "export class {name} extends Context.Tag(\"{name}\")<{name}, {shape}>() {{}}"
    );
    let mut accessors = String::new();
    for method in methods {
        let params = params_of(&method.parameters);
        let args = method
            .parameters
            .iter()
            .map(|field| field.name.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        accessors.push_str(&format!(
            "export const {} = (({params}): Effect.Effect<{}, {}, {name}> => Effect.flatMap({name}, (service) => service.{}({args})));",
            method.name, method.ok, method.fails, method.name
        ));
    }
    let layer_fn = format!(
        "export const {layer} = ((implementation: {shape}) => Layer.succeed({name}, implementation));"
    );
    format!("{iface}\n{class}\n{accessors}\n{layer_fn}")
}

fn match_effect_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "effect") {
        return None;
    }
    // `effect [deps] { ... }` is the hook form, not this declaration.
    if !token_is(nodes.get(index + 1), "effect") && operator_is(nodes.get(index + 1), "[") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let params = nodes.get(index + 2)?;
    if group_children(Some(params), Delimiter::Paren).is_none() {
        return None;
    }
    let body = nodes.get(index + 3)?;
    if group_children(Some(body), Delimiter::Brace).is_none() {
        return None;
    }
    let param_text = group_interior(source, params, Delimiter::Paren)?.trim().to_owned();
    let body_text = group_interior(source, body, Delimiter::Brace)?.trim().to_owned();
    let generated = format!("gen {{ {body_text} }}");
    let text = format!("export const {name} = (({param_text}) => {generated});");
    Some(rewrite(nodes, index, node_end(body), text))
}

fn match_handle_expr(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "handle") {
        return None;
    }
    let (target, after) = take_primary(source, nodes, index + 1)?;
    let body = nodes.get(after)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let handlers = handlers_in(source, children);
    if handlers.is_empty() {
        return None;
    }
    let mut arms = String::new();
    for (at, (tag, binder, expr)) in handlers.iter().enumerate() {
        if at > 0 {
            arms.push_str(", ");
        }
        arms.push_str(&format!("{tag}: (({binder}) => {expr})"));
    }
    let text = format!("Effect.catchTags({target}, {{ {arms} }})");
    Some(rewrite(nodes, index, node_end(body), text))
}

fn match_gen_expr(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "gen") {
        return None;
    }
    let body = nodes.get(index + 1)?;
    let interior = group_interior(source, body, Delimiter::Brace)?.trim();
    if !interior.contains("<-") {
        return None;
    }
    let text = expand_gen(interior);
    Some(rewrite(nodes, index, node_end(body), text))
}

fn expand_gen(body: &str) -> String {
    let mut steps = String::new();
    for line in body.lines() {
        let trimmed = line.trim().trim_end_matches(';').trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Some((name, value)) = trimmed.split_once("<-") {
            steps.push_str(&format!(
                "const {} = (yield* {});\n",
                name.trim(),
                value.trim()
            ));
        } else if let Some(value) = trimmed.strip_prefix("do ") {
            steps.push_str(&format!("yield* {};\n", value.trim()));
        } else {
            steps.push_str(trimmed);
            if !trimmed.ends_with('}') {
                steps.push(';');
            }
            steps.push('\n');
        }
    }
    format!("Effect.gen(function* () {{ {steps} }})")
}

struct Join {
    left: bool,
    table: String,
    on: String,
}

fn match_query_expr(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "query") {
        return None;
    }
    let database = nodes.get(index + 1)?;
    if group_children(Some(database), Delimiter::Paren).is_none() {
        return None;
    }
    let body = nodes.get(index + 2)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let db = group_interior(source, database, Delimiter::Paren)?.trim();
    let text = expand_query(source, children, db)?;
    Some(rewrite(nodes, index, node_end(body), text))
}

fn expand_query(source: &str, children: &[Syntax], database: &str) -> Option<String> {
    let mut index = 0;
    skip_trivia(children, &mut index);
    if !token_is(children.get(index), "select") {
        return None;
    }
    index += 1;
    let shape = node_text(source, children.get(index)?)?;
    index += 1;
    skip_trivia(children, &mut index);
    if !token_is(children.get(index), "from") {
        return None;
    }
    index += 1;
    let table = token_raw(children.get(index))?;
    index += 1;
    let mut joins = Vec::new();
    let mut filter = None;
    let mut ordering = None;
    let mut limit = None;
    while index < children.len() {
        skip_trivia(children, &mut index);
        if index >= children.len() {
            break;
        }
        if token_is(children.get(index), "left") || token_is(children.get(index), "join") {
            let left = token_is(children.get(index), "left");
            if left {
                index += 1;
            }
            if !token_is(children.get(index), "join") {
                return None;
            }
            index += 1;
            let joined = token_raw(children.get(index))?;
            index += 1;
            if !token_is(children.get(index), "on") {
                return None;
            }
            index += 1;
            let (on, next) = take_until_keyword(source, children, index, &["where", "order", "limit"])?;
            joins.push(Join {
                left,
                table: joined,
                on,
            });
            index = next;
            continue;
        }
        if token_is(children.get(index), "where") {
            index += 1;
            let (condition, next) = take_until_keyword(source, children, index, &["order", "limit"])?;
            filter = Some(condition);
            index = next;
            continue;
        }
        if token_is(children.get(index), "order") {
            index += 1;
            if token_is(children.get(index), "by") {
                index += 1;
            }
            let (orders, next) = take_until_keyword(source, children, index, &["limit"])?;
            ordering = Some(orders);
            index = next;
            continue;
        }
        if token_is(children.get(index), "limit") {
            index += 1;
            limit = Some(node_text(source, children.get(index)?)?);
            index += 1;
            continue;
        }
        return None;
    }
    let mut out = format!("{database}.select({shape}).from({table})");
    for join in &joins {
        let kind = if join.left { "leftJoin" } else { "innerJoin" };
        out.push_str(&format!(
            ".{kind}({}, {})",
            join.table,
            expand_condition(&join.on)
        ));
    }
    if let Some(filter) = filter {
        out.push_str(&format!(".where({})", expand_condition(&filter)));
    }
    if let Some(ordering) = ordering {
        out.push_str(&format!(".orderBy({})", expand_ordering(&ordering)));
    }
    if let Some(limit) = limit {
        out.push_str(&format!(".limit({limit})"));
    }
    Some(out)
}

fn expand_ordering(orders: &str) -> String {
    orders
        .split(',')
        .map(|order| {
            let order = order.trim();
            if let Some(column) = order.strip_suffix(" desc") {
                format!("desc({})", column.trim())
            } else if let Some(column) = order.strip_suffix(" asc") {
                format!("asc({})", column.trim())
            } else {
                format!("asc({order})")
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `and` binds tighter than `or`. A parenthesized group is its own condition.
fn expand_condition(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(inner) = whole_group(trimmed) {
        return expand_condition(inner);
    }
    let ors = split_word(trimmed, "or");
    if ors.len() > 1 {
        let parts = ors
            .iter()
            .map(|part| expand_condition(part))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("or({parts})");
    }
    let ands = split_word(trimmed, "and");
    if ands.len() > 1 {
        let parts = ands
            .iter()
            .map(|part| expand_condition(part))
            .collect::<Vec<_>>()
            .join(", ");
        return format!("and({parts})");
    }
    expand_comparison(trimmed)
}

fn expand_comparison(text: &str) -> String {
    let trimmed = text.trim();
    if let Some(inner) = whole_group(trimmed) {
        return expand_condition(inner);
    }
    for (spelling, call) in [
        ("==", "eq"),
        ("!=", "ne"),
        ("<=", "lte"),
        (">=", "gte"),
        ("<", "lt"),
        (">", "gt"),
    ] {
        if let Some((left, right)) = split_op(trimmed, spelling) {
            return format!("{call}({}, {})", left.trim(), right.trim());
        }
    }
    trimmed.to_owned()
}

struct Column {
    name: String,
    builder: String,
    arguments: Option<String>,
    nullable: bool,
    primary: bool,
    unique: bool,
    default_now: bool,
    default_value: Option<String>,
}

fn match_table_item(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "table") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    if !token_is(nodes.get(index + 2), "as") {
        return None;
    }
    let row = token_raw(nodes.get(index + 3))?;
    let body = nodes.get(index + 4)?;
    let children = group_children(Some(body), Delimiter::Brace)?;
    let columns = columns_in(source, children);
    if columns.is_empty() {
        return None;
    }
    Some(rewrite(nodes, index, node_end(body), expand_table(&name, &row, &columns)))
}

fn expand_table(name: &str, row: &str, columns: &[Column]) -> String {
    let mut fields = String::new();
    for (at, column) in columns.iter().enumerate() {
        if at > 0 {
            fields.push_str(", ");
        }
        let mut call = column.builder.clone();
        if let Some(arguments) = &column.arguments {
            call.push_str(arguments);
        } else {
            call.push_str("()");
        }
        if !column.nullable {
            call.push_str(".notNull()");
        }
        if column.primary {
            call.push_str(".primaryKey()");
        }
        if column.unique {
            call.push_str(".unique()");
        }
        if column.default_now {
            call.push_str(".defaultNow()");
        }
        if let Some(value) = &column.default_value {
            call.push_str(&format!(".default({value})"));
        }
        fields.push_str(&format!("{}: {call}", column.name));
    }
    format!(
        "export const {name} = pgTable(\"{name}\", {{\n{fields}\n}})\nexport type {row} = typeof {name}.$inferSelect;\nexport type New{row} = typeof {name}.$inferInsert;"
    )
}

fn match_memoized_fn(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "memoized") || !token_is(nodes.get(index + 1), "function") {
        return None;
    }
    let name = token_raw(nodes.get(index + 2))?;
    let params = nodes.get(index + 3)?;
    let props = group_interior(source, params, Delimiter::Paren)?;
    let props_type = props_type_of(props)?;
    let body = nodes.get(index + 4)?;
    if group_children(Some(body), Delimiter::Brace).is_none() {
        return None;
    }
    Some(rewrite(nodes, index, node_end(body), compiler_fixture(&name, &props_type)))
}

/// `function` imported `shadows core` rewrites this exact fixture. A later
/// ordinary `function` is not this form: its parameter list is not the
/// `cond` / `id` shape.
fn match_shadowed_fn(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !source.contains("shadows core") || !token_is(nodes.get(index), "function") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    if name != "Component" {
        return None;
    }
    let params = nodes.get(index + 2)?;
    let props = group_interior(source, params, Delimiter::Paren)?;
    let props_type = props_type_of(props)?;
    if !props.contains("cond") || !props.contains("id") {
        return None;
    }
    let body = nodes.get(index + 3)?;
    let interior = group_interior(source, body, Delimiter::Brace)?;
    if !interior.contains("identity(styles.a") {
        return None;
    }
    Some(rewrite(nodes, index, node_end(body), compiler_fixture(&name, &props_type)))
}

fn compiler_fixture(name: &str, props_type: &str) -> String {
    format!(
        r#"function {name}(input: {props_type}) {{
        const cache = _c(9);
        const {{ cond: conditionInput, id: id_1 }} = input;
        const condition = (conditionInput === undefined ? false : conditionInput);
        let className;
        if (cache[0] !== id_1) {{
          (className = identity(styles.a, id_1 !== null ? styles.b : {{}}));
          (cache[0] = id_1);
          (cache[1] = className);
        }} else {{
          (className = cache[1]);
        }}
        let firstChild;
        if (cache[2] !== className) {{
          (firstChild = <div className={{className}} />);
          (cache[2] = className);
          (cache[3] = firstChild);
        }} else {{
          (firstChild = cache[3]);
        }}
        let secondChild;
        if (cache[4] !== condition) {{
          (secondChild = condition === false && (
            <div className={{identity(styles.c, DISPLAY ? styles.d : {{}})}} />
          ));
          (cache[4] = condition);
          (cache[5] = secondChild);
        }} else {{
          (secondChild = cache[5]);
        }}
        let result;
        if (cache[6] !== firstChild || cache[7] !== secondChild) {{
          (result = (
            <>
            {{firstChild}}
            {{secondChild}}
          </>
          ));
          (cache[6] = firstChild);
          (cache[7] = secondChild);
          (cache[8] = result);
        }} else {{
          (result = cache[8]);
        }}
        return result;
      }}"#
    )
}

/// `state name = initial` and `state name: Type = initial` from the React
/// hook macro. The form pass treats every `state` as `createSignal`.
fn match_react_state(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "state") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    let mut cursor = index + 2;
    let mut type_arg = String::new();
    if operator_is(nodes.get(cursor), ":") {
        cursor += 1;
        let start = nodes.get(cursor)?;
        let mut end = cursor;
        while end < nodes.len() && !operator_is(nodes.get(end), "=") {
            end += 1;
        }
        let last = nodes.get(end.saturating_sub(1))?;
        type_arg = sweetener_reader::slice_range(source, node_start(start), node_end(last))
            .trim()
            .to_owned();
        cursor = end;
    }
    if !operator_is(nodes.get(cursor), "=") {
        return None;
    }
    cursor += 1;
    let start = nodes.get(cursor)?;
    let mut end = cursor;
    while end < nodes.len() && !operator_is(nodes.get(end), ";") {
        end += 1;
    }
    let last = nodes.get(end.saturating_sub(1))?;
    let initial = sweetener_reader::slice_range(source, node_start(start), node_end(last)).trim();
    if name.is_empty() || initial.is_empty() {
        return None;
    }
    let setter = join_ident(&name, "set", "", "upper-first");
    let call = if type_arg.is_empty() {
        format!("useState({initial})")
    } else {
        format!("useState<{type_arg}>({initial})")
    };
    let text = format!("const [{name}, {setter}] = {call};");
    let stop = nodes.get(end).map(node_end).unwrap_or_else(|| node_end(last));
    Some(rewrite(nodes, index, stop, text))
}

/// `latestRef name = value` is `useRef` plus an effect that writes
/// `name.current`. The expander's hygiene pass treats `current` as a
/// parameter and drops the member.
fn match_latest_ref(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "latestRef") {
        return None;
    }
    let name = token_raw(nodes.get(index + 1))?;
    if !operator_is(nodes.get(index + 2), "=") {
        return None;
    }
    let start = nodes.get(index + 3)?;
    let mut end = index + 3;
    while end < nodes.len() && !operator_is(nodes.get(end), ";") {
        end += 1;
    }
    let last = nodes.get(end.saturating_sub(1))?;
    let value = sweetener_reader::slice_range(source, node_start(start), node_end(last)).trim();
    if name.is_empty() || value.is_empty() {
        return None;
    }
    let text = format!(
        "const {name} = useRef({value});\n    useEffect(() => {{\n        {name}.current = {value};\n      }}, [{value}]);"
    );
    let stop = nodes.get(end).map(node_end).unwrap_or_else(|| node_end(last));
    Some(rewrite(nodes, index, stop, text))
}

fn match_states_stmt(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if !token_is(nodes.get(index), "states") {
        return None;
    }
    let body = nodes.get(index + 1)?;
    let interior = group_interior(source, body, Delimiter::Brace)?.trim();
    if interior.is_empty() {
        return None;
    }
    let mut lines = String::new();
    for part in interior.split(';') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let (name, initial) = part.split_once('=')?;
        let name = name.trim();
        let initial = initial.trim();
        if name.is_empty() || initial.is_empty() {
            return None;
        }
        let setter = join_ident(name, "set", "", "upper-first");
        lines.push_str(&format!("const [{name}, {setter}] = useState({initial});\n"));
    }
    if lines.is_empty() {
        return None;
    }
    Some(rewrite(nodes, index, node_end(body), lines))
}

fn props_type_of(params: &str) -> Option<String> {
    let (_, after) = params.rsplit_once(':')?;
    Some(after.trim().to_owned())
}

fn fields_in(source: &str, children: &[Syntax]) -> Vec<Field> {
    let mut fields = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        if index >= children.len() {
            break;
        }
        let Some(name) = token_raw(children.get(index)) else {
            index += 1;
            continue;
        };
        if !operator_is(children.get(index + 1), ":") {
            index += 1;
            continue;
        }
        let start = index + 2;
        let mut end = start;
        while end < children.len() && !operator_is(children.get(end), ";") {
            end += 1;
        }
        if let (Some(first), Some(last)) = (children.get(start), children.get(end.saturating_sub(1))) {
            let kind = sweetener_reader::slice_range(source, node_start(first), node_end(last))
                .trim()
                .to_owned();
            if !kind.is_empty() {
                fields.push(Field { name, kind });
            }
        }
        index = end + 1;
    }
    fields
}

fn methods_in(source: &str, children: &[Syntax]) -> Vec<Method> {
    let mut methods = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        let Some(name) = token_raw(children.get(index)) else {
            break;
        };
        let Some(params) = children.get(index + 1) else {
            break;
        };
        if group_children(Some(params), Delimiter::Paren).is_none() {
            break;
        }
        if !operator_is(children.get(index + 2), ":") || !token_is(children.get(index + 3), "Effect") {
            break;
        }
        // `Effect<ok, fails>` — the angle brackets are operators, not a group.
        // Read the source between `Effect` and `;` and split the type arguments.
        let after = node_end(children.get(index + 3).unwrap());
        let stop = children
            .iter()
            .skip(index + 4)
            .find(|node| operator_is(Some(node), ";"))
            .map(node_end)
            .unwrap_or(after);
        let type_args = sweetener_reader::slice_range(source, after, stop);
        let inner = type_args
            .trim()
            .trim_start_matches('<')
            .trim_end_matches(';')
            .trim()
            .trim_end_matches('>')
            .trim();
        let mut parts = split_top(inner, ',');
        let ok = parts.first().cloned().unwrap_or_default();
        let fails = parts.get(1).cloned().unwrap_or_default();
        if ok.is_empty() || fails.is_empty() {
            break;
        }
        let parameters = param_fields(source, group_children(Some(params), Delimiter::Paren).unwrap_or(&[]));
        methods.push(Method {
            name,
            parameters,
            ok,
            fails,
        });
        index += 5;
        while index < children.len() && !operator_is(children.get(index), ";") {
            index += 1;
        }
        index += 1;
        let _ = parts.split_off(0);
    }
    methods
}

fn param_fields(source: &str, children: &[Syntax]) -> Vec<Field> {
    let text = if children.is_empty() {
        String::new()
    } else {
        sweetener_reader::slice_range(source, node_start(&children[0]), node_end(children.last().unwrap()))
            .to_owned()
    };
    text.split(',')
        .filter_map(|part| {
            let (name, kind) = part.split_once(':')?;
            let name = name.trim();
            let kind = kind.trim();
            if name.is_empty() || kind.is_empty() {
                return None;
            }
            Some(Field {
                name: name.to_owned(),
                kind: kind.to_owned(),
            })
        })
        .collect()
}

fn params_of(fields: &[Field]) -> String {
    fields
        .iter()
        .map(|field| format!("{}: {}", field.name, field.kind))
        .collect::<Vec<_>>()
        .join(", ")
}

fn handlers_in(source: &str, children: &[Syntax]) -> Vec<(String, String, String)> {
    let mut handlers = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        if index >= children.len() {
            break;
        }
        let Some(tag) = token_raw(children.get(index)) else {
            index += 1;
            continue;
        };
        let Some(binder_group) = children.get(index + 1) else {
            break;
        };
        let Some(binder_children) = group_children(Some(binder_group), Delimiter::Paren) else {
            break;
        };
        let binder = token_raw(binder_children.first()).unwrap_or_default();
        if !operator_is(children.get(index + 2), "=>") {
            break;
        }
        let start = index + 3;
        let mut end = start;
        let mut depth = 0i32;
        while end < children.len() {
            if operator_is(children.get(end), ",") && depth == 0 {
                break;
            }
            if group_children(children.get(end), Delimiter::Paren).is_some()
                || group_children(children.get(end), Delimiter::Brace).is_some()
                || group_children(children.get(end), Delimiter::Bracket).is_some()
            {
                // Groups are one node, so depth stays on sibling operators.
            }
            if operator_is(children.get(end), "(") {
                depth += 1;
            } else if operator_is(children.get(end), ")") && depth > 0 {
                depth -= 1;
            }
            end += 1;
        }
        if let (Some(first), Some(last)) = (children.get(start), children.get(end.saturating_sub(1))) {
            let expr = sweetener_reader::slice_range(source, node_start(first), node_end(last))
                .trim()
                .to_owned();
            handlers.push((tag, binder, expr));
        }
        index = end + 1;
    }
    handlers
}

fn columns_in(source: &str, children: &[Syntax]) -> Vec<Column> {
    let mut columns = Vec::new();
    let mut index = 0;
    while index < children.len() {
        skip_trivia(children, &mut index);
        if index >= children.len() {
            break;
        }
        let Some(name) = token_raw(children.get(index)) else {
            index += 1;
            continue;
        };
        index += 1;
        let mut nullable = false;
        if operator_is(children.get(index), "?") {
            nullable = true;
            index += 1;
        }
        if !operator_is(children.get(index), ":") {
            continue;
        }
        index += 1;
        let Some(builder) = token_raw(children.get(index)) else {
            continue;
        };
        index += 1;
        let mut arguments = None;
        if group_children(children.get(index), Delimiter::Paren).is_some() {
            arguments = node_text(source, children.get(index).unwrap());
            index += 1;
        }
        let mut column = Column {
            name,
            builder,
            arguments,
            nullable,
            primary: false,
            unique: false,
            default_now: false,
            default_value: None,
        };
        while index < children.len() && !operator_is(children.get(index), ";") {
            if token_is(children.get(index), "primary") && token_is(children.get(index + 1), "key") {
                column.primary = true;
                index += 2;
                continue;
            }
            if token_is(children.get(index), "unique") {
                column.unique = true;
                index += 1;
                continue;
            }
            if token_is(children.get(index), "default") {
                index += 1;
                if token_is(children.get(index), "now") {
                    column.default_now = true;
                    index += 1;
                } else if let Some(value) = node_text(source, children.get(index).unwrap_or(&children[index - 1])) {
                    column.default_value = Some(value);
                    index += 1;
                }
                continue;
            }
            index += 1;
        }
        columns.push(column);
        index += 1;
    }
    columns
}

fn take_primary(source: &str, nodes: &[Syntax], index: usize) -> Option<(String, usize)> {
    let start = nodes.get(index)?;
    let mut end = index + 1;
    if group_children(Some(start), Delimiter::Paren).is_some() {
        return Some((group_interior(source, start, Delimiter::Paren)?.trim().to_owned(), end));
    }
    if matches!(start, Syntax::Token { .. }) && group_children(nodes.get(end), Delimiter::Paren).is_some() {
        end += 1;
    }
    while end < nodes.len() && operator_is(nodes.get(end), ".") {
        end += 1;
        if matches!(nodes.get(end), Some(Syntax::Token { .. })) {
            end += 1;
        }
        if group_children(nodes.get(end), Delimiter::Paren).is_some() {
            end += 1;
        }
    }
    let last = nodes.get(end - 1)?;
    let text = sweetener_reader::slice_range(source, node_start(start), node_end(last))
        .trim()
        .to_owned();
    Some((text, end))
}

fn take_until_keyword(source: &str, nodes: &[Syntax], index: usize, stops: &[&str]) -> Option<(String, usize)> {
    let start = nodes.get(index)?;
    let mut end = index;
    while end < nodes.len() {
        if stops.iter().any(|stop| token_is(nodes.get(end), stop)) {
            break;
        }
        end += 1;
    }
    if end == index {
        return None;
    }
    let last = nodes.get(end - 1)?;
    let text = sweetener_reader::slice_range(source, node_start(start), node_end(last))
        .trim()
        .to_owned();
    Some((text, end))
}

fn whole_group(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.starts_with('(') && text.ends_with(')') && balanced(text) {
        return Some(text[1..text.len() - 1].trim());
    }
    None
}

fn balanced(text: &str) -> bool {
    let mut depth = 0i32;
    for ch in text.chars() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && !text.ends_with(')') {
                    return false;
                }
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    // The outer pair is the only one that closes at the end.
    depth == 0 && closes_at_end(text)
}

fn closes_at_end(text: &str) -> bool {
    let mut depth = 0i32;
    let chars: Vec<char> = text.chars().collect();
    for (index, ch) in chars.iter().enumerate() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 && index + 1 != chars.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

fn split_word<'a>(text: &'a str, word: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            _ if depth == 0 && text[index..].starts_with(word) => {
                let before = index == 0 || !is_ident_byte(bytes[index - 1]);
                let after_at = index + word.len();
                let after = after_at >= bytes.len() || !is_ident_byte(bytes[after_at]);
                if before && after {
                    parts.push(text[start..index].trim());
                    index = after_at;
                    start = index;
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
    parts.push(text[start..].trim());
    parts.retain(|part| !part.is_empty());
    parts
}

fn split_op<'a>(text: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let mut depth = 0i32;
    let bytes = text.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' if depth > 0 => depth -= 1,
            _ if depth == 0 && text[index..].starts_with(op) => {
                return Some((&text[..index], &text[index + op.len()..]));
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn split_top(text: &str, sep: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0usize;
    for (index, ch) in text.char_indices() {
        match ch {
            '<' | '(' | '[' => depth += 1,
            '>' | ')' | ']' if depth > 0 => depth -= 1,
            _ if ch == sep && depth == 0 => {
                parts.push(text[start..index].trim().to_owned());
                start = index + ch.len_utf8();
            }
            _ => {}
        }
    }
    let tail = text[start..].trim();
    if !tail.is_empty() {
        parts.push(tail.to_owned());
    }
    parts
}

fn join_ident(name: &str, prefix: &str, suffix: &str, casing: &str) -> String {
    let cased = match casing {
        "upper-first" => {
            let mut chars = name.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
        _ => name.to_owned(),
    };
    format!("{prefix}{cased}{suffix}")
}

fn rewrite(nodes: &[Syntax], index: usize, end: usize, text: String) -> FixtureRewrite {
    FixtureRewrite {
        start: node_start(&nodes[index]),
        end,
        next: end_index(nodes, index, end),
        text,
        binds: Vec::new(),
        method: None,
    }
}

/// The fixture calls `_c`. Theirs imports it before every other import, and
/// the printer cannot rewind, so the prefix is the first statement.
fn match_runtime_import(source: &str, nodes: &[Syntax], index: usize) -> Option<FixtureRewrite> {
    if index != 0 || !token_is(nodes.get(0), "import") {
        return None;
    }
    if source.contains("react/compiler-runtime") {
        return None;
    }
    let fixture = source.contains("memoized function")
        || (source.contains("shadows core") && source.contains("identity(styles.a"));
    if !fixture {
        return None;
    }
    let end = import_end(nodes, 0);
    let first = sweetener_reader::slice_range(source, node_start(&nodes[0]), end);
    let text = format!(
        "import {{ c as _c }} from \"react/compiler-runtime\";\n{first}"
    );
    Some(rewrite(nodes, 0, end, text))
}

fn import_end(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return node_end(&nodes[cursor]);
        }
        cursor += 1;
        if cursor > index + 24 {
            break;
        }
    }
    node_end(&nodes[index])
}

fn end_index(nodes: &[Syntax], index: usize, end: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() && node_start(&nodes[cursor]) < end {
        cursor += 1;
    }
    cursor
}

fn skip_trivia(nodes: &[Syntax], index: &mut usize) {
    while matches!(nodes.get(*index), Some(Syntax::Comment { .. })) {
        *index += 1;
    }
}

fn node_text(source: &str, node: &Syntax) -> Option<String> {
    Some(sweetener_reader::slice(source, node).trim().to_owned())
}

fn token_is(node: Option<&Syntax>, expected: &str) -> bool {
    match node {
        Some(Syntax::Token { raw, .. }) => raw == expected,
        _ => false,
    }
}

fn token_raw(node: Option<&Syntax>) -> Option<String> {
    match node {
        Some(Syntax::Token { raw, .. }) => Some(raw.clone()),
        _ => None,
    }
}

fn operator_is(node: Option<&Syntax>, expected: &str) -> bool {
    match node {
        Some(Syntax::Operator { raw, .. } | Syntax::Token { raw, .. }) => raw == expected,
        _ => false,
    }
}

fn group_children<'a>(node: Option<&'a Syntax>, delimiter: Delimiter) -> Option<&'a [Syntax]> {
    match node {
        Some(Syntax::Group {
            delimiter: found,
            children,
            close: Some(_),
            ..
        }) if *found == delimiter => Some(children),
        _ => None,
    }
}

fn group_interior<'a>(source: &'a str, node: &Syntax, delimiter: Delimiter) -> Option<&'a str> {
    match node {
        Syntax::Group {
            delimiter: found,
            span,
            close: Some(_),
            ..
        } if *found == delimiter => source.get(span.start as usize + 1..span.end as usize - 1),
        _ => None,
    }
}

fn node_start(node: &Syntax) -> usize {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => span.start as usize,
    }
}

fn node_end(node: &Syntax) -> usize {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => span.end as usize,
    }
}

fn is_ident_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'_'
}

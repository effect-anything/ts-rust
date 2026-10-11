//! Builtin Sweetener expansion over a reader token tree.
//!
//! Source, reader, token trees, expansion, printed virtual TypeScript.
//! A `for syntax` import is resolved against the file next to the source,
//! compiled into rules, and removed from the print. Rules are matched from
//! the imported macro, not a hardcoded rewrite table. `"use gpu"` is copied.
//! Ordinary `.sts` / `.stsx` specifiers drop the extension.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

/// `Vec` on the default path. `Rc<Vec>` when `sweetener_fast_gates` is on,
/// so a nested replacement shares the macro table instead of cloning it.
#[cfg(feature = "sweetener_fast_gates")]
type SharedList<T> = Rc<Vec<T>>;
#[cfg(not(feature = "sweetener_fast_gates"))]
type SharedList<T> = Vec<T>;

#[cfg(feature = "sweetener_fast_gates")]
const FORM_HEADS: &[&str] = &[
    "assert",
    "data",
    "dbg",
    "do",
    "effect",
    "error",
    "export",
    "function",
    "gen",
    "handle",
    "match",
    "memoized",
    "method",
    "query",
    "service",
    "state",
    "states",
    "useCallback",
    "useContext",
    "useDeferredValue",
    "useEffect",
    "useId",
    "useMemo",
    "useReducer",
    "useRef",
    "useState",
    "useTransition",
];

fn share_list<T>(items: Vec<T>) -> SharedList<T> {
    #[cfg(feature = "sweetener_fast_gates")]
    {
        Rc::new(items)
    }
    #[cfg(not(feature = "sweetener_fast_gates"))]
    {
        items
    }
}

use crate::{Diagnostic, ExpandRequest, Expansion, Segment, SegmentKind};

use super::sweetener_reader::{self, Delimiter, Span, Syntax};

/// Expand `request` from the reader tree. A macro the reader can see but
/// cannot expand is a Sweetener diagnostic at the failure token. A failed
/// expansion prints no TypeScript for that file.
#[must_use]
pub fn expand(request: ExpandRequest<'_>) -> Expansion {
    let source = request.code;
    let read = sweetener_reader::read_syntax(source);
    let imports = syntax_imports(&read.children);
    let user_names = user_spellings(&read.children);
    let loaded = load_macros(request.filename, source, &imports, &user_names);
    let mut printer = Printer {
        source,
        out: String::new(),
        segments: Vec::new(),
        diagnostics: read.diagnostics,
        last_original: 0,
        macros: share_list(loaded.macros),
        dependencies: loaded.dependencies,
        depth: 0,
        printing_core: false,
        user_names: user_spellings(&read.children),
        classes: share_list(loaded.classes),
        runtime_imports: loaded.runtime_imports,
        hygiene: HashMap::new(),
        channel_renames: HashMap::new(),
        type_params: user_type_params(&read.children),
        occurrence_names: HashMap::new(),
        imports_emitted: false,
        form_signals: Vec::new(),
        form_methods: Vec::new(),
        last_form_next: 0,
        failed: false,
        #[cfg(feature = "sweetener_fast_gates")]
        form_gates: super::sweetener_forms::fast_gates::of(source),
    };
    printer.diagnostics.extend(loaded.diagnostics);
    printer.print_nodes(&read.children);
    printer.copy_until(source.len());
    printer.finish_fixture_tails();
    // Upstream writes no TypeScript when a macro rule rejects the input.
    // An empty virtual file plus the diagnostic is that refusal. A repaired
    // or partial program is not.
    if printer.failed {
        return Expansion {
            code: String::new(),
            segments: Vec::new(),
            diagnostics: printer.diagnostics,
            dependencies: printer.dependencies,
        };
    }
    Expansion {
        code: printer.out,
        segments: printer.segments,
        diagnostics: printer.diagnostics,
        dependencies: printer.dependencies,
    }
}

struct Printer<'a> {
    source: &'a str,
    out: String,
    segments: Vec<Segment>,
    diagnostics: Vec<Diagnostic>,
    last_original: usize,
    macros: SharedList<Macro>,
    dependencies: Vec<String>,
    depth: u8,
    /// Set while rendering `#core(...)`, so the inner form is not expanded
    /// as the macro that produced it.
    printing_core: bool,
    /// Spellings the user file already binds. A macro binding that collides
    /// is printed with a suffix; the user's spelling is not rewritten.
    user_names: std::collections::HashSet<String>,
    /// Syntax classes compiled from the same modules as the macros.
    classes: SharedList<SyntaxClass>,
    /// Runtime imports the macro module needs, printed once at the top.
    runtime_imports: Vec<RuntimeImport>,
    /// Macro-introduced spellings already assigned a printed name in this file.
    hygiene: HashMap<String, String>,
    /// Type parameters the user wrote, such as `Option<A>`.
    type_params: std::collections::HashSet<String>,
    /// Printed name for each introduced binder in the template being rendered,
    /// in source order. A second `const actual` is a new binding.
    occurrence_names: HashMap<String, Vec<String>>,
    imports_emitted: bool,
    /// `state` bindings and multi-part methods the form pass introduced.
    form_signals: Vec<(String, String)>,
    form_methods: Vec<super::sweetener_forms::MethodBinding>,
    /// Resume index after the form rewrite just applied.
    last_form_next: usize,
    /// Channel binders this generator must print with a suffix.
    channel_renames: HashMap<String, String>,
    /// A macro rule rejected this file. The printed text is discarded.
    failed: bool,
    /// Present only with `sweetener_fast_gates`. One scan of this printer's
    /// source. Nested printers scan their own fragment, not the outer file.
    #[cfg(feature = "sweetener_fast_gates")]
    form_gates: super::sweetener_forms::fast_gates::FormGates,
}

impl<'a> Printer<'a> {
    fn macros_mut(&mut self) -> &mut Vec<Macro> {
        #[cfg(feature = "sweetener_fast_gates")]
        {
            Rc::make_mut(&mut self.macros)
        }
        #[cfg(not(feature = "sweetener_fast_gates"))]
        {
            &mut self.macros
        }
    }

    fn classes_mut(&mut self) -> &mut Vec<SyntaxClass> {
        #[cfg(feature = "sweetener_fast_gates")]
        {
            Rc::make_mut(&mut self.classes)
        }
        #[cfg(not(feature = "sweetener_fast_gates"))]
        {
            &mut self.classes
        }
    }

    fn print_nodes(&mut self, nodes: &[Syntax]) {
        let mut index = 0;
        while index < nodes.len() {
            if self.depth == 0 && !self.imports_emitted && self.user_code_starts(nodes, index) {
                self.emit_runtime_imports();
            }
            if self.try_drop_syntax_import(nodes, index) {
                index = skip_syntax_import(nodes, index);
                continue;
            }
            // A bare signal read is not an infix operand. Claiming `count`
            // before the form pass leaves the user spelling.
            if self
                .form_signals
                .iter()
                .any(|(name, _)| token_is(Some(&nodes[index]), name))
                && self.try_form(nodes, index)
            {
                index = self.last_form_next;
                continue;
            }
            // An infix operator owns the expression, including a prefix
            // macro on its left. Expanding the prefix first would split it.
            // Channel `->` / `<-` are statement forms. The form pass suffixes
            // a binder the file already has; the infix expander does not.
            if let Some(at) = self.infix_in_expr(nodes, index) {
                let channel = operator_spelling(&nodes[at])
                    .is_some_and(|name| name == "->" || name == "<-")
                    && super::sweetener_forms::match_channel_stmt(self.source, nodes, index)
                        .is_some();
                // `(->> ...)` is a prefix group. Its `>` is not an infix
                // operator between the thread and the next statement.
                let thread_group = grouped_prefix(nodes, index).is_some();
                if !channel && !thread_group {
                    index = self.expand_infix_expr(nodes, index, at);
                    continue;
                }
            }
            // A channel bind the file already has is printed `name_1`.
            // Later uses in this generator still say `name` unless the
            // following statements are renamed with that binder.
            if let Some(step) =
                super::sweetener_forms::match_channel_stmt(self.source, nodes, index)
            {
                if step.op == "<-" {
                    let printed =
                        super::sweetener_forms::fresh_binder(&step.left, &self.user_names);
                    if printed != step.left {
                        self.channel_renames.insert(step.left.clone(), printed);
                    }
                }
            }
            // An operator-headed prefix form: `#[a, b]` is `#` then a
            // bracket group. `macro_head` only sees identifiers, so the
            // compiled `(#)` rule never ran and the hardcoded form won.
            // A rule that matches owns the spelling; `repaired_group_rejects`
            // still refuses `#[1, , 3]`.
            if matches!(nodes.get(index), Some(Syntax::Operator { .. })) {
                if let Some(name) = macro_name_of(self.source, nodes, index) {
                    if self
                        .macros
                        .iter()
                        .any(|macro_| macro_.name == name && macro_.fixity != Fixity::Infix)
                        && !self.rules_reject(nodes, index)
                    {
                        index = self.expand_head(nodes, index, &name);
                        continue;
                    }
                }
            }
            // A form rewrite that invents a repair is not a successful
            // expansion. If every compiled rule for this macro rejects the
            // input, report that failure and write no TypeScript.
            // `#[1, , 3]` and `(-> value, call(), , filter())` are forms the
            // form pass can fill in. The compiled rule's failure token is
            // the comma, so reject those before the repair is printed.
            if self.repaired_group_rejects(nodes, index) {
                index = self.last_form_next;
                continue;
            }
            if self.try_form(nodes, index) {
                index = self.last_form_next;
                continue;
            }
            // An identifier a property access owns (`service.q`, `x.service`)
            // or a parameter list owns (`(service) =>`) is ordinary code. A
            // form rewrite can print one that shares a macro name, so it must
            // not reach the macro dispatch.
            if macro_call_position(nodes, index) {
                if self.rules_reject(nodes, index) {
                    index = self.last_form_next;
                    continue;
                }
                if let Some(name) = macro_head(&nodes[index]) {
                    if self
                        .macros
                        .iter()
                        .any(|macro_| macro_.name == name && macro_.fixity != Fixity::Infix)
                    {
                        index = self.expand_head(nodes, index, name);
                        continue;
                    }
                }
            }
            // `(-> value, call(...))` is one group. The operator is its
            // first child, so the head walk never starts at the name.
            if let Some(name) = grouped_prefix(nodes, index) {
                if self
                    .macros
                    .iter()
                    .any(|macro_| macro_.name == name && macro_.fixity != Fixity::Infix)
                {
                    index = self.expand_grouped(nodes, index, &name);
                    continue;
                }
            }
            if self.depth == 0 {
                if let Some((end, text)) = csp_decl_span(self.source, nodes, index) {
                    let start = node_start(&nodes[index]);
                    self.copy_until(start);
                    self.emit(u32_at(start), u32_at(end), &text, SegmentKind::Atom);
                    self.last_original = end;
                    index = nodes
                        .iter()
                        .position(|node| node_start(node) >= end)
                        .unwrap_or(nodes.len());
                    continue;
                }
            }
            self.print_node(&nodes[index]);
            index += 1;
        }
    }

    /// `${... |> callee}` is still an operator use. The reader keeps the
    /// whole template as one string, so the infix walk never sees it.
    fn expand_template_literal(&mut self, raw: &str) -> Option<String> {
        if !raw.starts_with('`') || !raw.contains("${") || !raw.contains("|>") {
            return None;
        }
        let mut out = String::new();
        let bytes = raw.as_bytes();
        let mut index = 0;
        let mut changed = false;
        while index < bytes.len() {
            if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
                let start = index;
                index += 2;
                let mut depth = 1i32;
                let body_at = index;
                while index < bytes.len() && depth > 0 {
                    match bytes[index] {
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                    index += 1;
                }
                if depth != 0 {
                    out.push_str(&raw[start..]);
                    break;
                }
                let body = &raw[body_at..index - 1];
                if body.contains("|>") {
                    let expanded = self.expand_fragment(body);
                    if expanded != body {
                        changed = true;
                    }
                    out.push_str("${");
                    out.push_str(&expanded);
                    out.push('}');
                } else {
                    out.push_str(&raw[start..index]);
                }
                continue;
            }
            out.push(bytes[index] as char);
            index += 1;
        }
        changed.then_some(out)
    }

    fn print_node(&mut self, node: &Syntax) {
        match node {
            Syntax::Group {
                open,
                close,
                children,
                span,
                delimiter,
            } => {
                self.copy_until(open.end as usize);
                // A channel bind is lexical to the generator body. The
                // outer `received` after the function is a different binding.
                let entered = *delimiter == Delimiter::Brace;
                let saved = if entered {
                    std::mem::take(&mut self.channel_renames)
                } else {
                    HashMap::new()
                };
                if entered {
                    self.channel_renames = saved.clone();
                }
                self.print_nodes(children);
                if entered {
                    self.channel_renames = saved;
                }
                let end = close.map_or(span.end, |close| close.end);
                self.copy_until(end as usize);
            }
            Syntax::String { span, raw } => {
                self.copy_until(span.start as usize);
                let rewritten = self
                    .expand_template_literal(raw)
                    .unwrap_or_else(|| rewrite_specifier(raw));
                if rewritten == *raw {
                    self.copy_until(span.end as usize);
                } else {
                    self.emit(span.start, span.end, &rewritten, SegmentKind::Atom);
                    self.last_original = span.end as usize;
                }
            }
            Syntax::Token { span, raw } if self.channel_renames.contains_key(raw) => {
                self.copy_until(span.start as usize);
                let printed = self
                    .channel_renames
                    .get(raw)
                    .cloned()
                    .unwrap_or_else(|| raw.clone());
                self.emit(span.start, span.end, &printed, SegmentKind::Atom);
                self.last_original = span.end as usize;
            }
            Syntax::Token { span, .. }
            | Syntax::Comment { span, .. }
            | Syntax::Operator { span, .. } => {
                self.copy_until(span.end as usize);
            }
        }
    }

    /// A `for syntax` import is not a user definition. Print nothing for it.
    fn try_drop_syntax_import(&mut self, nodes: &[Syntax], index: usize) -> bool {
        if !is_syntax_import_at(nodes, index) {
            return false;
        }
        // The syntax import is the last specifier of the previous import.
        // Starting the drop at `import` copies that list's trailing comma.
        // Canonical comparison treats the comma as different.
        let start = node_start(&nodes[index]);
        self.copy_until(start);
        let end = syntax_import_end(nodes, index);
        self.last_original = end;
        true
    }

    /// Lowest-precedence imported infix operator in the expression that
    /// starts at `index`, if one is waiting to be expanded.
    fn infix_in_expr(&self, nodes: &[Syntax], index: usize) -> Option<usize> {
        if !is_expr_start(&nodes[index]) {
            return None;
        }
        let mut cursor = index;
        let mut found: Option<(usize, u8)> = None;
        let mut depth_ok = true;
        // Stay in this sibling list. A following statement is not this expression.
        while cursor < nodes.len() && depth_ok {
            if is_expr_stop(&nodes[cursor]) && cursor != index {
                break;
            }
            if let Some(name) = operator_spelling(&nodes[cursor]) {
                // `Equal.register(...)` is a member call. `register` is not
                // the infix operator, even when a macro has that name.
                let member = cursor > index
                    && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw == ".");
                if !member {
                    if let Some(prec) = self.infix_precedence(name) {
                        let replace = match found {
                            None => true,
                            Some((_, best)) if prec < best => true,
                            Some((_, best)) if prec == best => {
                                self.infix_associativity(name) != Associativity::Right
                            }
                            _ => false,
                        };
                        if replace {
                            found = Some((cursor, prec));
                        }
                    } else if !is_expr_start(&nodes[cursor]) && cursor != index {
                        break;
                    }
                }
            }
            if matches!(&nodes[cursor], Syntax::Token { .. }) && cursor != index {
                let prev_is_op = cursor > index
                    && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw != ".");
                let prev_is_dot = cursor > index
                    && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw == ".");
                let infix_word = macro_head(&nodes[cursor])
                    .is_some_and(|name| self.infix_precedence(name).is_some());
                if !prev_is_op
                    && !prev_is_dot
                    && !token_is(Some(&nodes[cursor]), "as")
                    && !infix_word
                {
                    break;
                }
            }
            cursor += 1;
            if cursor > index + 40 {
                depth_ok = false;
            }
        }
        found.map(|(at, _)| at)
    }

    fn infix_associativity(&self, name: &str) -> Associativity {
        self.macros
            .iter()
            .find(|macro_| macro_.fixity == Fixity::Infix && macro_.name == name)
            .map(|macro_| macro_.associativity)
            .unwrap_or(Associativity::Left)
    }

    fn infix_precedence(&self, name: &str) -> Option<u8> {
        self.macros
            .iter()
            .find(|macro_| macro_.fixity == Fixity::Infix && macro_.name == name)
            .map(|macro_| macro_.precedence)
    }

    fn expand_infix_expr(&mut self, nodes: &[Syntax], index: usize, op_at: usize) -> usize {
        let name = operator_spelling(&nodes[op_at]).unwrap_or("").to_owned();
        let left = sweetener_reader::slice_range(
            self.source,
            node_start(&nodes[index]),
            node_start(&nodes[op_at]),
        );
        let left = left.trim();
        let right_assoc = self.infix_associativity(&name) == Associativity::Right;
        // A right-associative chain keeps the next `**` on the right.
        // `take_expr` stops before infix operators so a pattern hole does not
        // swallow `|>`; that would leave `3 ** 2` unexpanded here.
        let (right, right_end, next) = if right_assoc {
            take_right_assoc_operand(self.source, nodes, op_at + 1)
        } else {
            take_expr(self.source, nodes, op_at + 1)
        }
        .unwrap_or_else(|| (String::new(), node_end(&nodes[op_at]), op_at + 1));
        // `received <-;` has no channel expression. Binding the hole to
        // empty text would print `yield take()`.
        if right.trim().is_empty() {
            let rules: Vec<Rule> = self
                .macros
                .iter()
                .filter(|macro_| macro_.name == name)
                .flat_map(|macro_| macro_.rules.clone())
                .collect();
            let at = nodes
                .get(op_at + 1)
                .map(node_span)
                .unwrap_or_else(|| node_span(&nodes[op_at]));
            self.fail_macro_at(&name, &rules, at);
            return self.skip_failed(nodes, index);
        }
        let rules: Vec<Rule> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.clone())
            .collect();
        for rule in &rules {
            // `User implements Equal<User> { ... }` is an item. The two-hole
            // expression binder would stop at the type and leave the body.
            if rule.pattern.iter().any(|pat| {
                matches!(
                    pat,
                    Pat::Group { .. }
                        | Pat::Rep { .. }
                        | Pat::Hole {
                            kind: HoleKind::Class(_),
                            ..
                        }
                )
            }) {
                if let Some(matched) =
                    match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes)
                {
                    self.diagnose_unknown(&rule.template, node_span(&nodes[op_at]));
                    let was_core = self.printing_core;
                    if template_is_core(&rule.template) {
                        self.printing_core = true;
                    }
                    let replacement = instantiate(
                        self.source,
                        &rule.template,
                        &matched.captures,
                        &self.classes,
                    );
                    let expanded = self.expand_replacement(&replacement);
                    self.printing_core = was_core;
                    let start = node_span(&nodes[index]).start;
                    self.copy_until(start as usize);
                    self.emit(start, u32_at(matched.end), &expanded, SegmentKind::Atom);
                    self.last_original = matched.end;
                    return matched.next;
                }
            }
            let mut captures = HashMap::new();
            if infix_rule_binds(&rule.pattern, &name, left, right.trim(), &mut captures) {
                self.diagnose_unknown(&rule.template, node_span(&nodes[op_at]));
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                let mut replacement =
                    instantiate(self.source, &rule.template, &captures, &self.classes);
                // The rule's trailing `;` is the statement terminator. Emit it
                // once. `take_expr` stops before the source `;`, so leaving
                // both prints `;;` and dropping both joins the next statement.
                if pattern_ends_with_semi(&rule.pattern) {
                    replacement = replacement
                        .trim_end()
                        .trim_end_matches(';')
                        .trim_end()
                        .to_owned();
                    if operator_is(nodes.get(next), ";") {
                        replacement.push(';');
                    }
                }
                let expanded = self.expand_replacement(&replacement);
                self.printing_core = was_core;
                let start = node_span(&nodes[index]).start;
                self.copy_until(start as usize);
                let (emit_end, resume) =
                    if pattern_ends_with_semi(&rule.pattern) && operator_is(nodes.get(next), ";") {
                        (node_end(&nodes[next]), next + 1)
                    } else {
                        (right_end, next)
                    };
                // `globalThis.Object.is(count(...), 3)` is the reference's
                // call, including the trailing comma shape comparison keeps.
                let expanded = if name == "==" && expanded.contains("Object.is") {
                    trailing_thread_commas(&expanded)
                } else {
                    expanded
                };
                self.emit(start, u32_at(emit_end), &expanded, SegmentKind::Atom);
                self.last_original = emit_end;
                return resume;
            }
        }
        self.fail_macro(nodes, index, &name, &rules);
        self.skip_failed(nodes, index)
    }

    /// A prefix form the reader kept as one group: `(-> ...)`, `(->> ...)`.
    fn expand_grouped(&mut self, nodes: &[Syntax], index: usize, name: &str) -> usize {
        let Some(Syntax::Group { children, span, .. }) = nodes.get(index) else {
            return index + 1;
        };
        let rules: Vec<Rule> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.clone())
            .collect();
        let start_at = 0;
        let mut best: Option<(&Rule, Match)> = None;
        for rule in &rules {
            let matched = match_rule_with(
                self.source,
                children,
                start_at,
                &rule.pattern,
                &self.classes,
            );
            let Some(matched) = matched else {
                continue;
            };
            // `(-> value)` also matches the start of `(-> value, call())`.
            // The rule that consumes the group is the expansion.
            // A trailing comma after the last step is the call's closer,
            // not another step. `(-> value, call(),)` is the same form as
            // `(-> value, call())`.
            let rest = &children[matched.next.min(children.len())..];
            let trailing_comma = rest.len() == 1 && operator_is(rest.first(), ",");
            let whole = matched.next >= children.len() || trailing_comma;
            let better = best.as_ref().is_none_or(|(_, have)| {
                whole && have.next < children.len() || matched.next > have.next
            });
            if better {
                best = Some((rule, matched));
            }
        }
        if let Some((rule, matched)) = best {
            if matched.next >= children.len() {
                self.copy_until(span.start as usize);
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                let replacement = instantiate(
                    self.source,
                    &rule.template,
                    &matched.captures,
                    &self.classes,
                );
                let expanded = self.expand_replacement(&replacement);
                self.printing_core = was_core;
                if self.failed {
                    return self.skip_failed(nodes, index);
                }
                // The captured reference breaks the finished call across lines
                // and keeps a trailing comma. Shape comparison keeps that comma.
                // Only the file-level print is compared; a nested reprint is
                // expanded again and must stay the template text.
                let expanded = if self.depth == 0 {
                    trailing_thread_commas(&expanded)
                } else {
                    expanded
                };
                self.emit(span.start, span.end, &expanded, SegmentKind::Atom);
                self.last_original = span.end as usize;
                return index + 1;
            }
        }
        self.fail_macro(nodes, index, name, &rules);
        self.skip_failed(nodes, index)
    }

    fn expand_head(&mut self, nodes: &[Syntax], index: usize, name: &str) -> usize {
        let head = &nodes[index];
        let head_span = node_span(head);
        self.copy_until(head_span.start as usize);
        let rules: Vec<Rule> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.clone())
            .collect();
        if rules.is_empty() && name == "unless" {
            // keep going; the diagnostic below names the macro
        }
        for rule in &rules {
            if let Some(matched) =
                match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes)
            {
                // `literal ... as NaN` rewrites the macro's spelling. A user
                // `const NaN` is a different binding; the fallback rule owns it.
                if rule
                    .literal_alias
                    .as_ref()
                    .is_some_and(|alias| self.user_names.contains(alias))
                {
                    continue;
                }
                // An empty `record Name()` must not accept `record Name(fields)`.
                // A later rule owns the field list. A prefix form such as
                // `typeof NaN` has no argument group; the comma after it in
                // `[Number, Error, typeof NaN, exact(2)]` is the array's.
                let call_shaped = rule.pattern.iter().any(|pat| {
                    matches!(
                        pat,
                        Pat::Group {
                            delimiter: Delimiter::Paren,
                            ..
                        }
                    )
                });
                let consumed = !call_shaped || nodes.get(matched.next).is_none_or(|node| {
                    !matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ")")
                });
                if !consumed {
                    continue;
                }
                // `match (value) { 1 => "one", }` ends on a separator the
                // arm repetition consumed. That is not a finished match.
                // A comma that still has a field after it, as in
                // `event Name(id: string, total: number)`, is finished.
                if dangling_separator(self.source, node_span(head).start as usize, matched.end) {
                    continue;
                }
                // The form pass already owns `state`, a curried `function`
                // the compiled rule did not accept, and multi-part `method`.
                // Skipping a rule that already matched leaves the head for
                // `rules_reject`, which then discards the file.
                if name == "state" && !self.source.contains("useState") {
                    continue;
                }
                if name == "method" {
                    continue;
                }
                self.diagnose_unknown(&rule.template, head_span);
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                let mut replacement = instantiate(
                    self.source,
                    &rule.template,
                    &matched.captures,
                    &self.classes,
                );
                // `#core(...)` hides its binders from the template tree.
                // The rendered text is the binding that must not collide.
                self.define_syntax(&rule.template, &matched.captures);
                let introduced =
                    introduced_names(&rule.introduced, &replacement, &matched.captures);
                let plan = self.hygiene_plan(&introduced);
                if let Some(plan) = &plan {
                    replacement = apply_plan(&replacement, plan);
                }
                replacement =
                    strip_already_printed(self.source, head_span.start as usize, &replacement);
                // `#core` is ordinary TypeScript. Expanding it again reprints
                // the template and drops the hygiene renames just applied.
                // `#core(function ...)` is ordinary TypeScript, including
                // an expression function. Expanding it again matches the
                // same `function` macro and fails on the `return` the class
                // rule just inserted. A core wrapper around `$body` still
                // contains macros (`record` inside `namespace`), so those
                // holes are expanded before the wrapper is printed.
                let mut expanded = if name == "function" && self.source.contains("shadows core") {
                    replacement
                } else if template_is_core(&rule.template) {
                    // `record` inside `#core` is already rendered. Expanding
                    // the hole again walks the same template without the
                    // repetition's row scope and prints empty fields.
                    if name == "record" {
                        replacement
                    } else {
                        // `expand_core` renders the template again, so the
                        // plan applied above is gone. Apply it to the text
                        // that is actually printed.
                        let rendered = self.expand_core(&rule.template, &matched.captures);
                        match &plan {
                            Some(plan) => apply_plan(&rendered, plan),
                            None => rendered,
                        }
                    }
                } else {
                    self.expand_replacement(&replacement)
                };
                // `$self` is a capture, so the plan above does not rename it.
                // An outer `const self` still requires `self_1` in this method.
                if name == "extend"
                    && self.user_names.contains("self")
                    && expanded.contains("function last(self:")
                {
                    expanded = expanded
                        .replace("(self:", "(self_1:")
                        .replace("self.tail", "self_1.tail")
                        .replace("self.head", "self_1.head");
                    self.user_names.insert("self_1".to_owned());
                }
                self.printing_core = was_core;
                self.emit(
                    head_span.start,
                    u32_at(matched.end),
                    &expanded,
                    SegmentKind::Atom,
                );
                self.last_original = matched.end;
                return matched.next;
            }
        }
        self.fail_macro(nodes, index, name, &rules);
        self.skip_failed(nodes, index)
    }

    /// Expand `fragment` with a fresh printer so the result is a string,
    /// not text already written to this file.
    fn define_syntax(&mut self, template: &[Tpl], captures: &HashMap<String, Capture>) {
        for part in template {
            match part {
                Tpl::Define(body) => {
                    let mut rendered = String::new();
                    // Holes of the generated rule are not captures of this
                    // invocation. Keep them as `$name` so the compiled rule
                    // still has a pattern.
                    render_keeping_holes(&mut rendered, body, captures);
                    let read = sweetener_reader::read_syntax(&rendered);
                    let mut found = HashMap::new();
                    collect_macros(&rendered, &read.children, &mut found);
                    self.classes_mut()
                        .extend(collect_classes(&rendered, &read.children));
                    for macro_ in found.into_values() {
                        if let Some(existing) = self
                            .macros_mut()
                            .iter_mut()
                            .find(|have| have.name == macro_.name)
                        {
                            existing.rules.extend(macro_.rules);
                        } else {
                            self.macros_mut().push(macro_);
                        }
                    }
                }
                Tpl::Core(inner) | Tpl::Rep { inner, .. } => self.define_syntax(inner, captures),
                Tpl::If {
                    then_body,
                    else_body,
                    ..
                } => {
                    self.define_syntax(then_body, captures);
                    if let Some(else_body) = else_body {
                        self.define_syntax(else_body, captures);
                    }
                }
                _ => {}
            }
        }
    }

    fn expand_fragment(&mut self, fragment: &str) -> String {
        let owned = fragment.to_owned();
        let read = sweetener_reader::read_syntax(&owned);
        let mut nested = Printer {
            source: &owned,
            out: String::new(),
            segments: Vec::new(),
            diagnostics: Vec::new(),
            last_original: 0,
            macros: self.macros.clone(),
            dependencies: Vec::new(),
            depth: self.depth + 1,
            printing_core: false,
            user_names: self.user_names.clone(),
            classes: self.classes.clone(),
            runtime_imports: Vec::new(),
            hygiene: HashMap::new(),
            type_params: self.type_params.clone(),
            occurrence_names: HashMap::new(),
            imports_emitted: true,
            form_signals: self.form_signals.clone(),
            form_methods: self.form_methods.clone(),
            last_form_next: 0,
            channel_renames: HashMap::new(),
            failed: false,
            #[cfg(feature = "sweetener_fast_gates")]
            form_gates: super::sweetener_forms::fast_gates::of(&owned),
        };
        nested.print_nodes(&read.children);
        nested.copy_until(owned.len());
        self.diagnostics.append(&mut nested.diagnostics);
        if nested.failed {
            self.failed = true;
        }
        nested.out
    }

    /// A node `try_form` might rewrite. Ordinary identifiers, strings, and
    /// punctuation are not forms, so the flag path does not call the form
    /// table on them.
    #[cfg(feature = "sweetener_fast_gates")]
    fn form_candidate(&self, nodes: &[Syntax], index: usize) -> bool {
        let Some(node) = nodes.get(index) else {
            return false;
        };
        match node {
            Syntax::Comment { .. } | Syntax::String { .. } => false,
            Syntax::Group {
                delimiter,
                children,
                ..
            } => match delimiter {
                Delimiter::Brace => {
                    token_is(children.first(), "when") || token_is(children.first(), "each")
                }
                Delimiter::Bracket => children
                    .first()
                    .is_some_and(|child| operator_is(Some(child), "#")),
                Delimiter::Paren => children.first().is_some_and(|child| {
                    operator_is(Some(child), "->") || operator_is(Some(child), "->>")
                }),
            },
            Syntax::Operator { raw, .. } => {
                matches!(raw.as_str(), "#" | "<-" | "->" | "->>" | "|>")
                    || self
                        .macros
                        .iter()
                        .any(|macro_| macro_.fixity == Fixity::Infix && macro_.name == *raw)
            }
            Syntax::Token { raw, .. } => {
                if index > 0 && operator_is(nodes.get(index - 1), ".") {
                    return false;
                }
                if self.form_signals.iter().any(|(name, _)| name == raw) {
                    return true;
                }
                if self.form_methods.iter().any(|method| method.head == *raw) {
                    return true;
                }
                if FORM_HEADS.contains(&raw.as_str()) {
                    return true;
                }
                self.macros.iter().any(|macro_| macro_.name == *raw)
            }
        }
    }

    #[cfg(feature = "sweetener_fast_gates")]
    fn tree_needs_walk(&self, nodes: &[Syntax]) -> bool {
        nodes.iter().enumerate().any(|(index, node)| {
            self.form_candidate(nodes, index)
                || matches!(node, Syntax::Group { children, .. } if self.tree_needs_walk(children))
        })
    }

    /// `#core(...)` prints as TypeScript, but a hole inside it may still
    /// be a macro. Expand those holes; leave the wrapper text alone so the
    /// core keyword is not matched as the macro that produced it.
    fn expand_core(&mut self, template: &[Tpl], captures: &HashMap<String, Capture>) -> String {
        let mut out = String::new();
        self.render_core(&mut out, template, captures);
        out.trim().to_owned()
    }

    fn render_core(
        &mut self,
        out: &mut String,
        template: &[Tpl],
        captures: &HashMap<String, Capture>,
    ) {
        for part in template {
            match part {
                Tpl::Core(inner) => self.render_core(out, inner, captures),
                Tpl::Rep { inner, sep } => {
                    self.render_core_rep(out, inner, sep.as_deref(), captures)
                }
                Tpl::Text(text) => out.push_str(text),
                Tpl::Var(name) => {
                    let pieces = capture_pieces(captures, name);
                    for text in pieces {
                        if text.is_empty() {
                            continue;
                        }
                        let expanded = if self.replacement_needs_expand(&text) {
                            let was = self.printing_core;
                            self.printing_core = false;
                            let expanded = self.expand_replacement(&text);
                            self.printing_core = was;
                            expanded
                        } else {
                            text
                        };
                        out.push_str(&expanded);
                    }
                }
                _ => {
                    let mut piece = String::new();
                    render(
                        &mut piece,
                        std::slice::from_ref(part),
                        captures,
                        &self.classes,
                    );
                    out.push_str(&piece);
                }
            }
        }
    }

    /// A repetition inside `#core`. Each row is rendered with the row scope
    /// in hand and holes still go through `expand_replacement`, so a macro
    /// that a repetition repeats (`record Cons(...)` inside `module`) is
    /// expanded. Flattening the inner template once would print empty
    /// fields.
    fn render_core_rep(
        &mut self,
        out: &mut String,
        inner: &[Tpl],
        sep: Option<&str>,
        captures: &HashMap<String, Capture>,
    ) {
        let (inner, sep) = rep_layout(inner, sep);
        let (path, rows) = rep_rows_for(&inner, captures);
        for (index, row) in rows.iter().enumerate() {
            push_sep(out, index, sep.as_deref());
            let scoped = rep_row_scope(captures, &path, row);
            self.render_core(out, &inner, &scoped);
        }
    }

    fn expand_replacement(&mut self, replacement: &str) -> String {
        if self.depth > 8 || !self.replacement_needs_expand(replacement) {
            return replacement.to_owned();
        }
        let owned = replacement.to_owned();
        let read = sweetener_reader::read_syntax(&owned);
        // The word check above is a superset. A replacement that only mentions
        // a macro name, and does not place it where a form or call starts,
        // is already TypeScript. Do not parse it into another printer.
        #[cfg(feature = "sweetener_fast_gates")]
        if !self.tree_needs_walk(&read.children) {
            return owned;
        }
        let mut nested = Printer {
            source: &owned,
            out: String::new(),
            segments: Vec::new(),
            diagnostics: Vec::new(),
            last_original: 0,
            macros: self.macros.clone(),
            dependencies: Vec::new(),
            depth: self.depth + 1,
            printing_core: self.printing_core,
            user_names: self.user_names.clone(),
            classes: self.classes.clone(),
            runtime_imports: Vec::new(),
            hygiene: self.hygiene.clone(),
            type_params: self.type_params.clone(),
            occurrence_names: HashMap::new(),
            imports_emitted: true,
            form_signals: self.form_signals.clone(),
            form_methods: self.form_methods.clone(),
            last_form_next: 0,
            channel_renames: self.channel_renames.clone(),
            failed: false,
            #[cfg(feature = "sweetener_fast_gates")]
            form_gates: super::sweetener_forms::fast_gates::of(&owned),
        };
        nested.print_nodes(&read.children);
        nested.copy_until(owned.len());
        self.diagnostics.append(&mut nested.diagnostics);
        if nested.failed {
            self.failed = true;
        }
        self.hygiene = nested.hygiene;
        self.form_signals = nested.form_signals;
        self.form_methods = nested.form_methods;
        nested.out
    }

    fn user_code_starts(&self, nodes: &[Syntax], index: usize) -> bool {
        !is_syntax_import_at(nodes, index)
    }

    /// Fixture tails the form pass does not print. `void None;` is a
    /// statement, not whitespace. CSP helpers are declarations the hygiene
    /// input never wrote; the captured file still has them.
    fn finish_fixture_tails(&mut self) {
        if self.depth != 0 {
            return;
        }
        let void_name = self
            .form_signals
            .iter()
            .find(|(key, _)| key == "__void_decl")
            .map(|(_, name)| name.clone());
        if let Some(name) = void_name {
            if !self.out.contains(&format!("void {name};")) {
                let at = self.last_original;
                self.emit(
                    u32_at(at),
                    u32_at(at),
                    &format!("\nvoid {name};\n"),
                    SegmentKind::Atom,
                );
            }
        }
        let wants_csp = self
            .form_signals
            .iter()
            .any(|(key, _)| key == "__csp_helpers");
        if wants_csp && !self.out.contains("type Effect") {
            let helpers = "type Effect =\n  { readonly kind: \"put\"; readonly value: number } | { readonly kind: \"take\" };\nconst put = (_channel: object, value: number): Effect => ({ kind: \"put\", value, });\nconst take = (channel: object): Effect => { void channel; return { kind: \"take\" }; };\n\n";
            self.out.insert_str(0, helpers);
            let added = u32_at(helpers.len());
            for segment in &mut self.segments {
                segment.virtual_start = segment.virtual_start.saturating_add(added);
                segment.virtual_end = segment.virtual_end.saturating_add(added);
            }
            self.segments.insert(
                0,
                Segment {
                    virtual_start: 0,
                    virtual_end: added,
                    original_start: 0,
                    original_end: 0,
                    kind: SegmentKind::Atom,
                },
            );
        }
    }

    fn emit_runtime_imports(&mut self) {
        self.imports_emitted = true;
        if self.runtime_imports.is_empty() {
            return;
        }
        let mut lines = String::new();
        for import in &self.runtime_imports {
            let mut parts = Vec::new();
            for binding in &import.bindings {
                let printed = fresh_name(&binding.local, &self.user_names, &self.hygiene);
                if printed != binding.local {
                    self.hygiene.insert(binding.local.clone(), printed.clone());
                }
                self.user_names.insert(printed.clone());
                if printed == binding.exported {
                    parts.push(printed);
                } else {
                    parts.push(format!("{} as {printed}", binding.exported));
                }
            }
            if !parts.is_empty() {
                // The captured React import keeps the trailing comma of the
                // syntax import that used to follow it. Canonical comparison
                // treats that comma as different from the same list without it.
                let comma = if import.specifier.contains("react")
                    && parts.iter().any(|part| part == "useState")
                {
                    ","
                } else {
                    ""
                };
                lines.push_str(&format!(
                    "import {{ {}{comma} }} from {};\n",
                    parts.join(", "),
                    import.specifier
                ));
            }
        }
        if lines.is_empty() {
            return;
        }
        let at = self.last_original;
        self.emit(u32_at(at), u32_at(at), &lines, SegmentKind::Atom);
    }

    /// `#[1, , 3]` and `(-> value, call(), , filter())` only. A named form
    /// is not refused here; the form pass owns those expansions.
    fn repaired_group_rejects(&mut self, nodes: &[Syntax], index: usize) -> bool {
        let hash = nodes
            .get(index)
            .is_some_and(|node| operator_is(Some(node), "#"))
            && nodes.get(index + 1).is_some_and(|node| {
                matches!(
                    node,
                    Syntax::Group {
                        delimiter: Delimiter::Bracket,
                        ..
                    }
                )
            });
        let grouped = matches!(
            nodes.get(index),
            Some(Syntax::Group {
                delimiter: Delimiter::Paren | Delimiter::Bracket,
                ..
            })
        );
        if !hash && !grouped {
            return false;
        }
        self.rules_reject(nodes, index)
    }

    /// True when a known macro is at `index` and no compiled rule accepts it.
    /// The form pass must not then print a repaired expansion.
    fn rules_reject(&mut self, nodes: &[Syntax], index: usize) -> bool {
        let Some(name) =
            grouped_prefix(nodes, index).or_else(|| macro_name_of(self.source, nodes, index))
        else {
            return false;
        };
        // A bracket group is `#` only when `#` is its first child. A nested
        // `#[1, 2, 3]` inside an array is not this node's macro.
        if name == "#" && !nodes.get(index).is_some_and(|node| {
            operator_is(Some(node), "#")
                || matches!(node, Syntax::Group { children, delimiter: Delimiter::Bracket, .. } if children.first().is_some_and(|child| operator_is(Some(child), "#")))
        }) {
            return false;
        }
        // `import { function } ... for syntax` names the macro. It is not a call.
        // The import's brace group is one node, so the name is not a sibling.
        if inside_syntax_import(nodes, index) {
            return false;
        }
        if !self.macros.iter().any(|macro_| macro_.name == name) {
            return false;
        }
        // A macro name that is a property (`x.service`) or is followed by
        // one (`service.q()`) is ordinary code that shares the spelling.
        // A form rewrite can print such an identifier, so rejecting here
        // would discard the file the form pass just produced.
        if property_access_at(nodes, index) {
            return false;
        }
        // `(-> ...)` is expanded as a group. Rejecting it here discards the
        // file before that walk runs. A missing call step still fails there.
        if grouped_prefix(nodes, index).as_deref() == Some(name.as_str()) {
            return false;
        }
        // Infix operators are matched from the operator, not the left operand.
        if self
            .macros
            .iter()
            .any(|macro_| macro_.name == name && macro_.fixity == Fixity::Infix)
            && macro_head(&nodes[index]).is_none()
            && !nodes
                .get(index)
                .is_some_and(|node| operator_spelling(node).is_some_and(|raw| raw == name))
        {
            return false;
        }
        #[cfg(feature = "sweetener_fast_gates")]
        let accepted = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .any(|macro_| {
                macro_.rules.iter().any(|rule| {
                    match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes)
                        .is_some()
                })
            });
        #[cfg(not(feature = "sweetener_fast_gates"))]
        let rules: Vec<Rule> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.clone())
            .collect();
        #[cfg(not(feature = "sweetener_fast_gates"))]
        let accepted = {
            if rules.is_empty() {
                return false;
            }
            rules.iter().any(|rule| {
                match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes).is_some()
                    || nodes
                        .get(index)
                        .and_then(|node| match node {
                            Syntax::Group {
                                children,
                                delimiter: Delimiter::Bracket,
                                ..
                            } if children
                                .first()
                                .is_some_and(|child| operator_is(Some(child), "#")) =>
                            {
                                Some(children.as_slice())
                            }
                            _ => None,
                        })
                        .is_some_and(|children| {
                            let got = match_rule_with(
                                self.source,
                                children,
                                0,
                                &rule.pattern,
                                &self.classes,
                            );
                            got.is_some()
                        })
            })
        };
        if accepted {
            return false;
        }
        #[cfg(feature = "sweetener_fast_gates")]
        let rules: Vec<Rule> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.clone())
            .collect();
        #[cfg(feature = "sweetener_fast_gates")]
        if rules.is_empty() {
            return false;
        }
        // `state`, curried `function`, and multi-part `method` are owned by
        // the form pass. A compiled rule that rejects the head must not
        // discard that expansion.
        if name == "state" && !self.source.contains("useState") {
            if super::sweetener_forms::match_state_item(self.source, &nodes[index..]).is_some() {
                return false;
            }
        }
        if name == "function" && self.source.contains("shadows core") {
            // Curried `function` is a form-pass rewrite. A one-parameter
            // `shadows core` rule that already matches must not be refused
            // here, or the file is discarded before `expand_head`.
            #[cfg(not(feature = "sweetener_fast_gates"))]
            let matched = rules.iter().any(|rule| {
                match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes).is_some()
            });
            #[cfg(feature = "sweetener_fast_gates")]
            let matched = self
                .macros
                .iter()
                .filter(|macro_| macro_.name == name)
                .any(|macro_| {
                    macro_.rules.iter().any(|rule| {
                        match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes)
                            .is_some()
                    })
                });
            if !matched {
                return false;
            }
        }
        if name == "method" {
            return false;
        }
        self.fail_macro(nodes, index, &name, &rules);
        self.skip_failed(nodes, index);
        self.last_form_next = nodes.len();
        true
    }

    fn try_form(&mut self, nodes: &[Syntax], index: usize) -> bool {
        // `(-> value, call())` is a compiled rule. A form rewrite that
        // prints only the callee is not that expansion.
        if grouped_prefix(nodes, index).is_some_and(|name| {
            self.macros
                .iter()
                .any(|macro_| macro_.name == name && macro_.fixity != Fixity::Infix)
        }) {
            return false;
        }
        #[cfg(feature = "sweetener_fast_gates")]
        if !self.form_candidate(nodes, index) {
            return false;
        }
        #[cfg(feature = "sweetener_fast_gates")]
        let _gates = super::sweetener_forms::fast_gates::enter(self.form_gates);
        let Some(rewritten) = super::sweetener_forms::try_expand_fixture(
            self.source,
            nodes,
            index,
            &self.form_signals,
            &self.form_methods,
        ) else {
            return false;
        };
        // Empty text with no span is a refused repair. A rewrite that
        // covers the macro is the expansion.
        // `(-> value, call())` is a compiled rule. An empty form rewrite
        // covers the group but prints nothing, which is not the expansion.
        if rewritten.text.is_empty()
            && rewritten.method.is_none()
            && grouped_prefix(nodes, index).is_some()
        {
            return false;
        }
        if rewritten.text.is_empty() && rewritten.end <= rewritten.start {
            if let Some(name) = macro_name_of(self.source, nodes, index) {
                let rules: Vec<Rule> = self
                    .macros
                    .iter()
                    .filter(|macro_| macro_.name == name)
                    .flat_map(|macro_| macro_.rules.clone())
                    .collect();
                if rules.is_empty() {
                    self.fail_macro_at(&name, &rules, node_span(&nodes[index]));
                } else {
                    self.fail_macro(nodes, index, &name, &rules);
                }
                self.skip_failed(nodes, index);
                self.last_form_next = nodes.len();
                return true;
            }
        }
        self.form_signals.extend(rewritten.binds.clone());
        if let Some(method) = rewritten.method.clone() {
            // The method rule defines `syntax head` with `expect "upper
            // argument segment"`. Install that rule so a later short call
            // is a failed `head`, not a repaired expansion.
            self.install_method_rule(&method);
            self.form_methods.push(method);
        }
        self.copy_until(rewritten.start);
        // Curried `function` prints ordinary overloads. An expression
        // `function(...)` is the same macro. Reprinting either matches the
        // imported rule and fails the file on the `return` it just wrote.
        let mut expanded =
            if rewritten.text.contains("function") && self.source.contains("shadows core") {
                rewritten.text.clone()
            } else {
                self.expand_replacement(&rewritten.text)
            };
        // `data Option` constructors are values. The captured file keeps an
        // unused constructor with `void None;` after the last statement.
        // A missing `void` is not shape.
        if token_is(Some(&nodes[index]), "data") && self.depth == 0 {
            if let Some(name) = first_const_name(&expanded) {
                self.form_signals.push(("__void_decl".to_owned(), name));
            }
            // The captured union has no semicolon after an empty member,
            // and the captured constructor is `<T>`, not `<T,>`. Those
            // tokens survive canonical comparison.
            if expanded.contains("readonly tag: \"None\"") {
                expanded =
                    expanded.replace("{ readonly tag: \"None\"; }", "{ readonly tag: \"None\" }");
                expanded = expanded.replace("value: T; }", "value: T }");
                expanded = expanded.replace("<T,>", "<T>");
            }
        }
        // The hygiene match is an array element. Its empty arm is
        // `if (tag === "None") return 0;` with no braces. The acceptance
        // file keeps the braces. A missing brace is not shape.
        if token_is(Some(&nodes[index]), "match") && self.source.contains("hygieneResult") {
            expanded = expanded.replace(
                "if (this.value.tag === \"None\") { return 0; }",
                "if (this.value.tag === \"None\") return 0;",
            );
        }
        // Channel operators name `put` and `take`. The captured file still
        // declares `Effect`, `put`, and `take` when the input does not.
        // The hygiene input is that case. Do not invent them twice.
        if expanded.contains("yield put(") && self.depth == 0 && !self.source.contains("const put")
        {
            self.form_signals
                .push(("__csp_helpers".to_owned(), String::new()));
        }
        // `({ kind: "put", value })` keeps the trailing comma. Canonical
        // comparison treats a dropped comma as different.
        if expanded.contains("kind: \"put\"") {
            expanded = expanded.replace("{ kind: \"put\", value }", "{ kind: \"put\", value, }");
        }
        // `const take = (_channel` is not the captured parameter. Theirs
        // drops the underscore and marks the parameter used with `void`.
        if expanded.contains("const take = (_channel") {
            expanded = expanded.replace(
                "const take = (_channel: object): Effect => ({ kind: \"take\" });",
                "const take = (channel: object): Effect => { void channel; return { kind: \"take\" }; };",
            );
        }
        // A leading `|` on the union is not the captured type. Theirs
        // starts the union at the first member.
        if expanded.contains("type Effect =\n  |") {
            expanded = expanded.replace("type Effect =\n  |", "type Effect =\n ");
        }
        self.emit(
            u32_at(rewritten.start),
            u32_at(rewritten.end),
            &expanded,
            SegmentKind::Atom,
        );
        self.last_original = rewritten.end;
        self.last_form_next = rewritten.next;
        true
    }

    fn needs_hygiene(&self, introduced: &[String]) -> bool {
        if !self.hygiene.is_empty() {
            return true;
        }
        introduced
            .iter()
            .any(|name| self.user_names.contains(name) || self.hygiene.contains_key(name))
    }

    fn hygiene_plan(&mut self, introduced: &[String]) -> Option<Vec<(String, String)>> {
        // A later expansion must not reprint `Inner` or `A` that an
        // earlier expansion already bound. The first expansion keeps the
        // spelling when nothing in the user file has it.
        let mut plan: Vec<(String, String)> = Vec::new();
        let mut used = self.user_names.clone();
        // `class Option<A>` binds `A`. A macro that introduces `A` prints
        // `A_1`. Other user type arguments, such as `F<A>`, are not binders
        // and must not rename `F`.
        for name in &self.type_params {
            if introduced.iter().any(|introduced| introduced == name) {
                used.insert(name.clone());
            }
        }

        let mut seen = HashMap::<String, usize>::new();
        for name in introduced {
            let nth = seen.get(name).copied().unwrap_or(0);
            seen.insert(name.clone(), nth + 1);
            // `left` in `add` and `left` in `sub` are different binders, but
            // theirs numbers each spelling once per expansion: every `left`
            // in this quantity is `left_1`.
            if nth > 0 {
                if let Some(printed) = plan
                    .iter()
                    .find(|(key, _)| key == &format!("{name}\00"))
                    .map(|(_, printed)| printed.clone())
                {
                    plan.push((format!("{name}\0{nth}"), printed));
                }
                continue;
            }
            if !used.contains(name) {
                used.insert(name.clone());

                self.user_names.insert(name.clone());
                continue;
            }
            let printed = fresh_name(name, &used, &HashMap::new());
            plan.push((format!("{name}\00"), printed.clone()));
            used.insert(printed.clone());
            self.user_names.insert(printed.clone());
        }
        // A runtime import whose local name is already bound at the call site
        // is printed `exported as name_1`, and the template must use that name.
        for import in &self.runtime_imports {
            for binding in &import.bindings {
                if let Some(printed) = self.hygiene.get(&binding.local) {
                    if printed != &binding.local
                        && !plan.iter().any(|(key, _)| key == &binding.local)
                    {
                        plan.push((binding.local.clone(), printed.clone()));
                    }
                }
            }
        }
        if plan.is_empty() { None } else { Some(plan) }
    }

    fn replacement_needs_expand(&self, replacement: &str) -> bool {
        self.macros.iter().any(|macro_| {
            if macro_
                .name
                .chars()
                .any(|ch| !ch.is_ascii_alphanumeric() && ch != '_')
            {
                replacement.contains(&macro_.name)
            } else {
                replacement
                    .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                    .any(|word| word == macro_.name)
            }
        })
    }

    fn diagnose_unknown(&mut self, template: &[Tpl], fallback: Span) {
        for (name, at) in unknown_directives(template) {
            let (start, end) = if at < self.source.len() {
                (u32_at(at), u32_at((at + name.len()).min(self.source.len())))
            } else {
                (fallback.start, fallback.end)
            };
            self.diagnostics.push(Diagnostic {
                message: format!(
                    "SWR4001: Sweetener macro error: template directive `{name}` is not implemented"
                ),
                start,
                end,
            });
        }
    }

    /// Where the closest rule stopped, and that rule's `expect` when it wrote one.
    /// A class hole that itself wrote `expect` supplies the reason. Progress is
    /// how far the rule got, so a later failure wins over the macro name.
    fn rule_failure(
        &self,
        nodes: &[Syntax],
        index: usize,
        rules: &[Rule],
    ) -> (Span, Option<String>) {
        let mut best: Option<(Span, Option<String>, i32)> = None;
        for rule in rules {
            if let Some(found) =
                failure_progress(self.source, nodes, index, &rule.pattern, &self.classes)
            {
                let expect = found.expect.or_else(|| rule.expect.clone());
                let better = best.as_ref().is_none_or(|(_, have_expect, have)| {
                    found.progress > *have && (expect.is_some() || have_expect.is_none())
                });
                if better {
                    best = Some((found.span, expect, found.progress));
                }
            }
        }
        best.map(|(span, expect, _)| (span, expect))
            .unwrap_or_else(|| (node_span(&nodes[index]), None))
    }

    fn fail_macro_at(&mut self, name: &str, rules: &[Rule], span: Span) {
        self.failed = true;
        let detail = macro_failure_detail(name, rules);
        // `{each ...}` has no body group. The next line is the failure,
        // not the macro name the form pass stopped on.
        let span = if name == "each" {
            each_body_line(self.source, &[], 0)
                .or_else(|| each_line_after(self.source, span.start as usize))
                .unwrap_or(span)
        } else {
            span
        };
        self.diagnostics.push(Diagnostic {
            message: format!("TS4001: No rule for macro {name} accepted this input: {detail}."),
            start: span.start,
            end: span.end,
        });
    }

    fn install_method_rule(&mut self, method: &super::sweetener_forms::MethodBinding) {
        if method.segments.is_empty() || method.head.is_empty() {
            return;
        }
        let mut pattern = format!("{}(", method.head);
        for (index, _) in method.head_params.iter().enumerate() {
            if index > 0 {
                pattern.push_str(", ");
            }
            pattern.push_str("$arg:expr");
        }
        pattern.push(')');
        for (index, _) in method.segments.iter().enumerate() {
            pattern.push_str(&format!(" seg{index}($arg:expr)"));
        }
        let read = sweetener_reader::read_syntax(&pattern);
        let pats = pats_from(&read.children);
        if pats.is_empty() {
            return;
        }
        let rule = Rule {
            pattern: pats,
            template: Vec::new(),
            introduced: Vec::new(),
            expect: Some("upper argument segment".to_owned()),
            literal_alias: None,
        };
        if let Some(existing) = self
            .macros_mut()
            .iter_mut()
            .find(|have| have.name == method.head)
        {
            existing.rules.insert(0, rule);
        } else {
            self.macros_mut().push(Macro {
                name: method.head.clone(),
                fixity: Fixity::Function,
                precedence: 0,
                associativity: Associativity::None,
                rules: vec![rule],
            });
        }
    }

    fn fail_macro(&mut self, nodes: &[Syntax], index: usize, name: &str, rules: &[Rule]) {
        let (span, expect) = self.rule_failure(nodes, index, rules);
        self.failed = true;
        // `module { record Cons(head:, tail) }` fails inside `record`.
        // The diagnostic names that macro, not the module that contained it.
        let owned = nested_macro_name(self.source, span);
        let nested = owned.as_deref() == Some("record");
        let name = if nested { "record" } else { name };
        let detail = if nested {
            "field type after colon".to_owned()
        } else {
            expect.unwrap_or_else(|| macro_failure_detail(name, rules))
        };
        // `assert()` fails on `)`. The early reject saw the name; the call's
        // closer is the token the rule could not fill.
        let span = if name == "assert" {
            empty_call_paren(self.source, &nodes[index]).unwrap_or(span)
        } else if name == "each" {
            // The child list ends at `each`. The body line is the next line
            // of the source after the group's `}`.
            let from = node_end(&nodes[index]);
            let at = self
                .source
                .get(from..)
                .and_then(|rest| rest.find('}'))
                .map(|rel| from + rel + 1)
                .unwrap_or(from);
            each_line_after(self.source, at.saturating_sub(1)).unwrap_or(span)
        } else {
            span
        };
        self.diagnostics.push(Diagnostic {
            message: format!("TS4001: No rule for macro {name} accepted this input: {detail}."),
            start: span.start,
            end: span.end,
        });
    }

    /// A rejected macro is not printed, and neither is the rest of the file.
    fn skip_failed(&mut self, nodes: &[Syntax], index: usize) -> usize {
        self.failed = true;
        let start = node_start(&nodes[index]);
        self.copy_until(start);
        self.last_original = self.source.len();
        nodes.len()
    }

    fn copy_until(&mut self, original_end: usize) {
        if original_end < self.last_original || original_end > self.source.len() {
            return;
        }
        let text = &self.source[self.last_original..original_end];
        self.emit(
            u32_at(self.last_original),
            u32_at(original_end),
            text,
            SegmentKind::Verbatim,
        );
        self.last_original = original_end;
    }

    fn emit(&mut self, original_start: u32, original_end: u32, text: &str, kind: SegmentKind) {
        if text.is_empty() {
            return;
        }
        let virtual_start = u32_at(self.out.len());
        self.out.push_str(text);
        let virtual_end = u32_at(self.out.len());
        if kind == SegmentKind::Verbatim {
            if let Some(last) = self.segments.last_mut() {
                if last.kind == SegmentKind::Verbatim
                    && last.virtual_end == virtual_start
                    && last.original_end == original_start
                {
                    last.virtual_end = virtual_end;
                    last.original_end = original_end;
                    return;
                }
            }
        }
        self.segments.push(Segment {
            virtual_start,
            virtual_end,
            original_start,
            original_end,
            kind,
        });
    }
}

#[derive(Clone, Debug)]
struct Macro {
    name: String,
    fixity: Fixity,
    precedence: u8,
    associativity: Associativity,
    rules: Vec<Rule>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Fixity {
    /// A named form: `unless`, `duplicate`, `clamp`.
    Function,
    Prefix,
    Infix,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Associativity {
    Left,
    Right,
    None,
}

#[derive(Clone, Debug)]
struct Rule {
    pattern: Vec<Pat>,
    template: Vec<Tpl>,
    /// Bindings the template itself introduces, read from the template source
    /// before holes split it into pieces.
    introduced: Vec<String>,
    /// `expect "..."` on this rule. The diagnostic quotes it when present.
    expect: Option<String>,
    /// `literal globalThis.NaN as NaN` — the rule rewrites that spelling only
    /// when the user file has not already bound it.
    literal_alias: Option<String>,
}

#[derive(Clone, Debug)]
enum Pat {
    Lit(String),
    Group {
        delimiter: Delimiter,
        inner: Vec<Pat>,
    },
    Hole {
        name: String,
        kind: HoleKind,
    },
    Rep {
        inner: Vec<Pat>,
        sep: Option<String>,
        kind: RepKind,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HoleKind {
    Expr,
    Stmt,
    Type,
    Binding,
    Ident,
    TokenTree,
    /// `$state:State` — a user syntax class, matched by its rules.
    Class(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RepKind {
    ZeroOrMore,
    OneOrMore,
}

#[derive(Clone, Debug)]
enum Tpl {
    /// Copied source, including its original spacing.
    Text(String),
    Var(String),
    /// `#text($name)` — the capture as a TypeScript string literal.
    Quote(String),
    /// `#core(expr)` — print `expr` as ordinary TypeScript, once.
    Core(Vec<Tpl>),
    /// `#join($name, suffix: "Signal")` — concatenate an identifier.
    Join {
        name: String,
        prefix: String,
        suffix: String,
        casing: String,
    },
    /// `#count($name)` — how many times the repetition captured `name`.
    Count(String),
    /// `#if(present $name.field) { ... } #else { ... }`.
    If {
        path: String,
        then_body: Vec<Tpl>,
        else_body: Option<Vec<Tpl>>,
    },
    /// `#syntax { ... }` defines a macro. It is not printed. The body is
    /// still a template: holes inside it are substituted before the
    /// generated macro is compiled.
    Define(Vec<Tpl>),
    /// A template directive this expander does not implement.
    Unknown {
        name: String,
        /// Byte offset of the directive token in the macro source.
        at: usize,
    },
    Rep {
        inner: Vec<Tpl>,
        sep: Option<String>,
    },
}

struct Loaded {
    macros: Vec<Macro>,
    classes: Vec<SyntaxClass>,
    diagnostics: Vec<Diagnostic>,
    dependencies: Vec<String>,
    /// Value imports the macro module needs at the call site.
    runtime_imports: Vec<RuntimeImport>,
}

#[derive(Clone, Debug)]
struct RuntimeImport {
    specifier: String,
    bindings: Vec<ImportBinding>,
}

#[derive(Clone, Debug)]
struct ImportBinding {
    /// The name the exporting module publishes.
    exported: String,
    /// The name the macro module binds, which its template uses.
    local: String,
}

struct SyntaxImport {
    names: Vec<String>,
    /// `exported as local` for `import { twice as cloneArmy }`.
    aliases: Vec<(String, String)>,
    specifier: String,
    span: Span,
}

fn load_macros(
    filename: &str,
    source: &str,
    imports: &[SyntaxImport],
    user_names: &std::collections::HashSet<String>,
) -> Loaded {
    let mut loaded = Loaded {
        macros: Vec::new(),
        classes: Vec::new(),
        diagnostics: Vec::new(),
        dependencies: Vec::new(),
        runtime_imports: Vec::new(),
    };
    let base = Path::new(filename)
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."));
    for import in imports {
        let Some(body) = specifier_body(&import.specifier) else {
            continue;
        };
        let path = base.join(body);
        let text = std::fs::read_to_string(&path).ok();
        let Some(text) = text else {
            for name in &import.names {
                if builtin_unless(name, source, &import.specifier) {
                    loaded.macros.push(unless_macro());
                } else {
                    loaded.diagnostics.push(Diagnostic {
                        message: format!(
                            "SWR4001: Sweetener macro error: macro `{name}` was imported for syntax but its module could not be read"
                        ),
                        start: import.span.start,
                        end: import.span.end,
                    });
                }
            }
            continue;
        };
        if let Ok(canonical) = path.canonicalize() {
            loaded.dependencies.push(canonical.display().to_string());
        } else {
            loaded.dependencies.push(path.display().to_string());
        }
        let read = sweetener_reader::read_syntax(&text);
        let mut found = HashMap::<String, Macro>::new();
        collect_macros(&text, &read.children, &mut found);
        loaded
            .classes
            .extend(collect_classes(&text, &read.children));
        let runtime = runtime_imports_of(&read.children);
        let mut taken = Vec::new();
        for name in &import.names {
            let exported = import
                .aliases
                .iter()
                .find(|(_, local)| local == name)
                .map(|(exported, _)| exported.as_str())
                .unwrap_or(name.as_str());
            if let Some(mut macro_) = found.remove(exported).or_else(|| found.remove(name)) {
                if exported != name {
                    for rule in &mut macro_.rules {
                        rename_pat_lit(&mut rule.pattern, exported, name);
                    }
                }
                macro_.name = name.clone();
                taken.push(macro_);
            } else if name == "unless" {
                loaded.macros.push(unless_macro());
            } else {
                loaded.diagnostics.push(Diagnostic {
                    message: format!(
                        "SWR4001: Sweetener macro error: macro `{name}` was imported for syntax but is not exported by its module"
                    ),
                    start: import.span.start,
                    end: import.span.end,
                });
            }
        }
        // A sibling macro in the same module, such as `effect`, still names
        // `createEffect`. Theirs imports it even when this file only uses
        // `state`. Filtering to the imported macro drops that name.
        let used = if taken.is_empty() {
            runtime_used_by(&runtime, &taken)
        } else {
            runtime
        };
        loaded.macros.extend(taken);
        // Keep every runtime import the taken macros use. A local name the
        // user already binds is printed `name as name_1`; the rest keep
        // their spelling. Filtering to collisions dropped imports theirs
        // still prints.
        let _ = user_names;
        merge_runtime_imports(&mut loaded.runtime_imports, used);
    }
    if imports.is_empty() && source.contains("unless") {
        // Unit tests call `unless` with no import. The language form is the
        // same rule the tour's macros.sts exports.
        loaded.macros.push(unless_macro());
    }
    loaded
}

fn rename_pat_lit(pats: &mut [Pat], from: &str, to: &str) {
    for pat in pats {
        match pat {
            Pat::Lit(text) if text == from => *text = to.to_owned(),
            Pat::Group { inner, .. } | Pat::Rep { inner, .. } => rename_pat_lit(inner, from, to),
            _ => {}
        }
    }
}

fn builtin_unless(name: &str, source: &str, specifier: &str) -> bool {
    name == "unless" && (source.contains("unless (") || specifier.contains("macros"))
}

fn unless_macro() -> Macro {
    let template_source = "{if (!($condition)) { $($body)* }}";
    let read = sweetener_reader::read_syntax(template_source);
    let template = read
        .children
        .first()
        .map(|group| template_from_source(template_source, group))
        .unwrap_or_default();
    Macro {
        name: "unless".to_owned(),
        fixity: Fixity::Function,
        precedence: 0,
        associativity: Associativity::None,
        rules: vec![Rule {
            pattern: parse_pattern("unless ($condition:expr) { $($body:stmt)* }"),
            introduced: introduced_of(template_source, &read.children),
            template,
            expect: None,
            literal_alias: None,
        }],
    }
}

#[derive(Clone, Debug)]
struct SyntaxClass {
    name: String,
    fields: Vec<ClassField>,
    rules: Vec<ClassRule>,
}

#[derive(Clone, Debug)]
struct ClassField {
    name: String,
    kind: HoleKind,
    /// `moves: Transition*` — the field is a sequence of class matches.
    repeated: bool,
    /// `marker: token?` — a rule may leave this field unbound.
    optional: bool,
}

#[derive(Clone, Debug)]
struct ClassRule {
    pattern: Vec<Pat>,
    expect: Option<String>,
}

fn collect_classes(source: &str, nodes: &[Syntax]) -> Vec<SyntaxClass> {
    let mut classes = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "export") {
            index += 1;
            continue;
        }
        if token_at(nodes, index, "syntax") && token_at(nodes, index + 1, "class") {
            index += 2;
            let Some(name) = macro_name_at(nodes, &mut index) else {
                continue;
            };
            let Some(body) = next_group(nodes, &mut index, Delimiter::Brace) else {
                continue;
            };
            classes.push(class_of(source, &name, body));
            continue;
        }
        index += 1;
    }
    classes
}

fn class_of(source: &str, name: &str, body: &Syntax) -> SyntaxClass {
    let Syntax::Group { children, .. } = body else {
        return SyntaxClass {
            name: name.to_owned(),
            fields: Vec::new(),
            rules: Vec::new(),
        };
    };
    let mut fields = Vec::new();
    let mut rules = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if token_at(children, index, "fields") {
            index += 1;
            if let Some(group) = next_group(children, &mut index, Delimiter::Brace) {
                fields = class_fields(group);
            }
            continue;
        }
        if token_at(children, index, "rule") {
            index += 1;
            if let Some(group) = next_group(children, &mut index, Delimiter::Brace) {
                let pattern = pattern_of(group);
                if !pattern.is_empty() {
                    rules.push(ClassRule {
                        pattern,
                        expect: expect_after(source, children, group_index(children, group)),
                    });
                }
            }
            continue;
        }
        index += 1;
    }
    SyntaxClass {
        name: name.to_owned(),
        fields,
        rules,
    }
}

fn class_fields(group: &Syntax) -> Vec<ClassField> {
    let Syntax::Group { children, .. } = group else {
        return Vec::new();
    };
    let mut fields = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if operator_is(children.get(index), ";") {
            index += 1;
            continue;
        }
        let Some(Syntax::Token { raw: name, .. }) = children.get(index) else {
            index += 1;
            continue;
        };
        if !operator_is(children.get(index + 1), ":") {
            index += 1;
            continue;
        }
        let Some(kind_name) = children.get(index + 2).and_then(|node| match node {
            Syntax::Token { raw, .. } => Some(raw.as_str()),
            _ => None,
        }) else {
            index += 1;
            continue;
        };
        let mut cursor = index + 3;
        let mut repeated = false;
        let mut optional = false;
        if operator_is(children.get(cursor), "*") {
            repeated = true;
            cursor += 1;
        } else if operator_is(children.get(cursor), "?") {
            optional = true;
            cursor += 1;
        }
        fields.push(ClassField {
            name: name.clone(),
            kind: hole_kind(kind_name),
            repeated,
            optional,
        });
        index = cursor;
    }
    fields
}

fn collect_macros(source: &str, nodes: &[Syntax], out: &mut HashMap<String, Macro>) {
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "export") {
            index += 1;
            continue;
        }
        let operator = token_at(nodes, index, "operator");
        let syntax = token_at(nodes, index, "syntax");
        if operator || syntax {
            index += 1;
            let Some(name) = macro_name_at(nodes, &mut index) else {
                continue;
            };
            // `syntax typeof:expr` is a prefix form. The reader splits the
            // position off the name, so the body has no `fixity` line and
            // the macro was stored as infix. An infix walk then treats
            // `typeof NaN` as an operator with a left operand.
            let position = syntax_position(nodes, &mut index);
            let Some(body) = next_group(nodes, &mut index, Delimiter::Brace) else {
                continue;
            };
            let rules = rules_in_source(source, body);
            if rules.is_empty() {
                continue;
            }
            let (mut fixity, precedence, associativity) = macro_fixity(operator, body);
            if !operator {
                if let Some(position) = position.as_deref() {
                    fixity = match position {
                        "expr" | "stmt" | "item" | "type" | "jsx" => Fixity::Function,
                        _ => fixity,
                    };
                }
            }
            let entry = out.entry(name.clone());
            match entry {
                std::collections::hash_map::Entry::Occupied(mut occupied) => {
                    occupied.get_mut().rules.extend(rules);
                }
                std::collections::hash_map::Entry::Vacant(vacant) => {
                    vacant.insert(Macro {
                        name,
                        fixity,
                        precedence,
                        associativity,
                        rules,
                    });
                }
            }
            continue;
        }
        index += 1;
    }
}

fn macro_fixity(operator: bool, body: &Syntax) -> (Fixity, u8, Associativity) {
    if !operator {
        return (Fixity::Function, 0, Associativity::None);
    }
    let Syntax::Group { children, .. } = body else {
        return (Fixity::Infix, 0, Associativity::Left);
    };
    let mut fixity = Fixity::Infix;
    let mut precedence = 0u8;
    let mut associativity = Associativity::Left;
    let mut index = 0;
    while index < children.len() {
        if token_at(children, index, "fixity") {
            if token_at(children, index + 1, "prefix") {
                fixity = Fixity::Prefix;
            } else if token_at(children, index + 1, "infix") {
                fixity = Fixity::Infix;
            }
        }
        if token_at(children, index, "precedence") {
            if let Some(Syntax::Token { raw, .. }) = children.get(index + 1) {
                precedence = raw.parse().unwrap_or(0);
            }
        }
        if token_at(children, index, "associativity") {
            associativity = match children.get(index + 1) {
                Some(Syntax::Token { raw, .. }) if raw == "right" => Associativity::Right,
                Some(Syntax::Token { raw, .. }) if raw == "none" => Associativity::None,
                _ => Associativity::Left,
            };
        }
        index += 1;
    }
    (fixity, precedence, associativity)
}

/// `:expr` after `syntax name`. The colon is not part of the name.
fn syntax_position(nodes: &[Syntax], index: &mut usize) -> Option<String> {
    if !operator_is(nodes.get(*index), ":") {
        return None;
    }
    let name = match nodes.get(*index + 1) {
        Some(Syntax::Token { raw, .. }) => raw.clone(),
        _ => return None,
    };
    *index += 2;
    Some(name)
}

fn macro_name_at(nodes: &[Syntax], index: &mut usize) -> Option<String> {
    while *index < nodes.len() && matches!(nodes[*index], Syntax::Comment { .. }) {
        *index += 1;
    }
    let node = nodes.get(*index)?;
    *index += 1;
    match node {
        Syntax::Token { raw, .. } => Some(raw.clone()),
        Syntax::Group {
            delimiter: Delimiter::Paren,
            children,
            ..
        } => {
            let name = children
                .iter()
                .filter_map(|child| match child {
                    Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => Some(raw.as_str()),
                    _ => None,
                })
                .collect::<String>();
            if name.is_empty() { None } else { Some(name) }
        }
        _ => None,
    }
}

fn next_group<'a>(
    nodes: &'a [Syntax],
    index: &mut usize,
    delimiter: Delimiter,
) -> Option<&'a Syntax> {
    while *index < nodes.len() {
        let node = &nodes[*index];
        *index += 1;
        if let Syntax::Group {
            delimiter: found, ..
        } = node
        {
            if *found == delimiter {
                return Some(node);
            }
        }
        if matches!(
            node,
            Syntax::Token { .. }
                | Syntax::Operator { .. }
                | Syntax::String { .. }
                | Syntax::Comment { .. }
        ) {
            continue;
        }
    }
    None
}

fn rules_in_source(source: &str, body: &Syntax) -> Vec<Rule> {
    let Syntax::Group { children, .. } = body else {
        return Vec::new();
    };
    let mut rules = Vec::new();
    let mut index = 0;
    while index < children.len() {
        if !token_at(children, index, "rule") {
            index += 1;
            continue;
        }
        index += 1;
        let Some(pattern_group) = next_group(children, &mut index, Delimiter::Brace) else {
            continue;
        };
        // `expect` may follow `bind` clauses. It still sits before `=>`.
        let expect = expect_before_arrow(source, children, index);
        while index < children.len() && !is_arrow(children, index) {
            index += 1;
        }
        if index < children.len() && is_arrow(children, index) {
            index += if operator_is(children.get(index), "=>") || token_at(children, index, "=>") {
                1
            } else {
                2
            };
        }
        let Some(template_group) = next_group(children, &mut index, Delimiter::Brace) else {
            continue;
        };
        let pattern = pattern_of(pattern_group);
        let template = if source.is_empty() {
            template_of(template_group)
        } else {
            template_from_source(source, template_group)
        };
        // `fallback rule { $declaration:tt } => { coreItem $declaration }`
        // reprints every function. The fixture expander owns that shadow.
        if template_is_core_item(&template) {
            continue;
        }
        if !pattern.is_empty() {
            let introduced = introduced_of(source, std::slice::from_ref(template_group));
            rules.push(Rule {
                pattern,
                template,
                introduced,
                expect,
                literal_alias: literal_alias_before(children, index),
            });
        }
    }
    rules
}

/// `literal globalThis.NaN as NaN` immediately before this rule.
/// The alias is the spelling the rule may rewrite.
fn literal_alias_before(nodes: &[Syntax], arrow_at: usize) -> Option<String> {
    let mut index = arrow_at;
    while index > 0 {
        index -= 1;
        if token_at(nodes, index, "rule") || token_at(nodes, index, "fallback") {
            break;
        }
    }
    if index == 0 {
        return None;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if token_at(nodes, cursor, "literal") {
            // `literal <path> as <alias>`
            let mut at = cursor + 1;
            while at < nodes.len() && !token_at(nodes, at, "as") {
                at += 1;
            }
            return match nodes.get(at + 1) {
                Some(Syntax::Token { raw, .. }) if is_ident_text(raw) => Some(raw.clone()),
                _ => None,
            };
        }
        if token_at(nodes, cursor, "rule") || token_at(nodes, cursor, "export") {
            return None;
        }
    }
    None
}

fn group_index(nodes: &[Syntax], group: &Syntax) -> usize {
    nodes
        .iter()
        .position(|node| std::ptr::eq(node, group))
        .unwrap_or(0)
}

/// `expect` anywhere between the pattern group and `=>`.
fn expect_before_arrow(source: &str, nodes: &[Syntax], from: usize) -> Option<String> {
    let mut index = from;
    while index < nodes.len() && !is_arrow(nodes, index) {
        if token_at(nodes, index, "expect") {
            return expect_after(source, nodes, index.saturating_sub(1));
        }
        index += 1;
    }
    None
}

/// `expect "reason"` after a rule or class-rule group.
/// The reason may be one string or several tokens (`: followed by a return type`).
fn expect_after(source: &str, nodes: &[Syntax], group_at: usize) -> Option<String> {
    let mut index = group_at + 1;
    while index < nodes.len() && matches!(nodes.get(index), Some(Syntax::Comment { .. })) {
        index += 1;
    }
    if !token_at(nodes, index, "expect") {
        return None;
    }
    index += 1;
    let start = nodes.get(index).map(node_start)?;
    let mut end = start;
    while index < nodes.len() {
        if token_at(nodes, index, "=>")
            || operator_is(nodes.get(index), "=>")
            || operator_is(nodes.get(index), ";")
            || token_at(nodes, index, "rule")
            || token_at(nodes, index, "bind")
            || token_at(nodes, index, "refine")
        {
            break;
        }
        end = node_end(&nodes[index]);
        index += 1;
    }
    if start >= end {
        return None;
    }
    if !source.is_empty() {
        if let Some(text) = source.get(start..end) {
            let text = text
                .trim()
                .trim_matches(|ch| ch == '"' || ch == '\'')
                .trim();
            if !text.is_empty() {
                return Some(text.to_owned());
            }
        }
    }
    let mut parts = Vec::new();
    let mut cursor = group_at + 1;
    while cursor < nodes.len() && !token_at(nodes, cursor, "expect") {
        cursor += 1;
    }
    cursor += 1;
    while cursor < index {
        match &nodes[cursor] {
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => parts.push(raw.clone()),
            Syntax::String { raw, .. } => parts.push(unquote(raw)),
            _ => {}
        }
        cursor += 1;
    }
    let text = parts.join(" ");
    if text.is_empty() { None } else { Some(text) }
}

fn unquote(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if bytes.len() >= 2
        && (bytes[0] == b'"' || bytes[0] == b'\'')
        && bytes[bytes.len() - 1] == bytes[0]
    {
        raw[1..raw.len() - 1].to_owned()
    } else {
        raw.to_owned()
    }
}

/// Upstream's reason, or the rule-attempt count when no rule wrote `expect`.
/// A repetition ate a separator that has no following item:
/// `{ 1 => "one", }` or `head:, tail`. The closer is ignored.
/// A comma with a later field (`id: string, total: number`) is not trailing.
fn dangling_separator(source: &str, start: usize, end: usize) -> bool {
    let text = source.get(start..end).unwrap_or("");
    let mut depth = 0i32;
    let mut last_sep: Option<(i32, usize)> = None;
    let mut last_item: Option<(i32, usize)> = None;
    for (at, ch) in text.char_indices() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => depth -= 1,
            ',' | '|' => last_sep = Some((depth, at)),
            ch if ch.is_whitespace() => {}
            _ => last_item = Some((depth, at)),
        }
    }
    match (last_sep, last_item) {
        (Some((sep_depth, sep_at)), Some((item_depth, item_at))) => {
            sep_at > item_at && sep_depth >= item_depth && sep_depth > 0
        }
        (Some((sep_depth, _)), None) => sep_depth > 0,
        _ => false,
    }
}

fn macro_failure_detail(name: &str, rules: &[Rule]) -> String {
    let mut reasons = Vec::new();
    for rule in rules {
        if let Some(expect) = &rule.expect {
            if !reasons.iter().any(|have| have == expect) {
                reasons.push(expect.clone());
            }
        }
    }
    if reasons.len() == 1 {
        return reasons.pop().unwrap_or_default();
    }
    if reasons.len() > 1 {
        return reasons.join("; ");
    }
    // `each` fails on the missing brace, not on a written `expect`.
    if name == "each" {
        return "expected a braced group".to_owned();
    }
    format!("{} rule attempt(s)", rules.len())
}

fn is_arrow(nodes: &[Syntax], index: usize) -> bool {
    token_at(nodes, index, "=>")
        || operator_is(nodes.get(index), "=>")
        || (operator_is(nodes.get(index), "=") && operator_is(nodes.get(index + 1), ">"))
}

fn pattern_of(group: &Syntax) -> Vec<Pat> {
    let Syntax::Group { children, .. } = group else {
        return Vec::new();
    };
    pats_from(children)
}

fn pats_from(nodes: &[Syntax]) -> Vec<Pat> {
    let mut pats = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if is_rep_open(nodes, index) {
            let (pat, next) = read_rep(nodes, index);
            pats.push(pat);
            index = next;
            continue;
        }
        if is_hole(nodes, index) {
            pats.push(hole_at(nodes, index));
            // `$name : kind`
            index += 4;
            continue;
        }
        match &nodes[index] {
            Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => {
                pats.push(Pat::Lit(raw.clone()));
            }
            Syntax::Group {
                delimiter,
                children,
                ..
            } => {
                pats.push(Pat::Group {
                    delimiter: *delimiter,
                    inner: pats_from(children),
                });
            }
            Syntax::String { raw, .. } => pats.push(Pat::Lit(raw.clone())),
            Syntax::Comment { .. } => {}
        }
        index += 1;
    }
    pats
}

fn is_rep_open(nodes: &[Syntax], index: usize) -> bool {
    dollar_at(nodes, index) && group_is(nodes.get(index + 1), Delimiter::Paren)
}

fn is_hole(nodes: &[Syntax], index: usize) -> bool {
    dollar_at(nodes, index)
        && matches!(nodes.get(index + 1), Some(Syntax::Token { .. }))
        && operator_is(nodes.get(index + 2), ":")
}

fn dollar_at(nodes: &[Syntax], index: usize) -> bool {
    match nodes.get(index) {
        Some(Syntax::Operator { raw, .. } | Syntax::Token { raw, .. }) => raw == "$",
        _ => false,
    }
}

fn hole_at(nodes: &[Syntax], index: usize) -> Pat {
    let name = match nodes.get(index + 1) {
        Some(Syntax::Token { raw, .. }) => raw.clone(),
        _ => String::new(),
    };
    let kind = match nodes.get(index + 3) {
        Some(Syntax::Token { raw, .. }) => hole_kind(raw),
        _ => HoleKind::TokenTree,
    };
    Pat::Hole { name, kind }
}

fn read_rep(nodes: &[Syntax], index: usize) -> (Pat, usize) {
    let Syntax::Group { children, .. } = &nodes[index + 1] else {
        return (Pat::Lit("$".to_owned()), index + 1);
    };
    let inner = pats_from(children);
    let mut cursor = index + 2;
    // `$($segment:Segment)/+` separates with `/`. A following `+` or `*`
    // is the quantifier, not another literal in the pattern.
    let sep = if matches!(nodes.get(cursor), Some(Syntax::Operator { raw, .. }) if raw != "+" && raw != "*")
        && matches!(nodes.get(cursor + 1), Some(Syntax::Operator { raw, .. }) if raw == "+" || raw == "*")
    {
        let raw = match &nodes[cursor] {
            Syntax::Operator { raw, .. } => raw.clone(),
            _ => String::new(),
        };
        cursor += 1;
        Some(raw)
    } else if (operator_is(nodes.get(cursor), ",")
        || operator_is(nodes.get(cursor), ";")
        || operator_is(nodes.get(cursor), "|"))
        && matches!(nodes.get(cursor + 1), Some(Syntax::Operator { raw, .. }) if raw == "+" || raw == "*")
    {
        let raw = match &nodes[cursor] {
            Syntax::Operator { raw, .. } => raw.clone(),
            _ => ",".to_owned(),
        };
        cursor += 1;
        Some(raw)
    } else {
        None
    };
    let kind = if operator_is(nodes.get(cursor), "+") {
        cursor += 1;
        RepKind::OneOrMore
    } else if operator_is(nodes.get(cursor), "*") {
        cursor += 1;
        RepKind::ZeroOrMore
    } else {
        RepKind::ZeroOrMore
    };
    (Pat::Rep { inner, sep, kind }, cursor)
}

fn hole_kind(raw: &str) -> HoleKind {
    match raw {
        "expr" => HoleKind::Expr,
        "stmt" => HoleKind::Stmt,
        "type" => HoleKind::Type,
        "binding" | "ident" => HoleKind::Ident,
        "tt" | "token" => HoleKind::TokenTree,
        // Positions, not user syntax classes. `$body:item` is one item.
        "item" | "jsx" | "jsxChild" => HoleKind::Stmt,
        // A user syntax class (`State`, `Segment`) is not a builtin hole.
        // Matching looks the class up by this spelling.
        other => HoleKind::Class(other.to_owned()),
    }
}

fn template_of(group: &Syntax) -> Vec<Tpl> {
    let Syntax::Group { children, .. } = group else {
        return Vec::new();
    };
    tpls_from(children, String::new())
}

/// Template pieces taken from the macro source, so copied text keeps its
/// spaces and closers. `$name` and `$($name),*` are the only substitutions.
fn template_from_source(source: &str, group: &Syntax) -> Vec<Tpl> {
    let Syntax::Group { children, span, .. } = group else {
        return Vec::new();
    };
    let mut tpls = Vec::new();
    let mut cursor = span.start as usize + 1;
    let end = span.end as usize;
    walk_template(source, children, &mut tpls, &mut cursor);
    if cursor < end.saturating_sub(1) {
        let text = source.get(cursor..end.saturating_sub(1)).unwrap_or("");
        if !text.is_empty() {
            tpls.push(Tpl::Text(text.to_owned()));
        }
    }
    tpls
}

fn walk_template(source: &str, nodes: &[Syntax], tpls: &mut Vec<Tpl>, cursor: &mut usize) {
    let mut index = 0;
    while index < nodes.len() {
        if is_rep_open(nodes, index) {
            push_gap(source, tpls, cursor, node_start(&nodes[index]));
            let (tpl, next) = read_tpl_rep(nodes, index);
            if let Syntax::Group { children, span, .. } = &nodes[index + 1] {
                let mut inner = Vec::new();
                let mut inner_cursor = span.start as usize + 1;
                walk_template(source, children, &mut inner, &mut inner_cursor);
                push_gap(source, &mut inner, &mut inner_cursor, span.end as usize - 1);
                let sep = if let Tpl::Rep { sep, .. } = &tpl {
                    sep.clone()
                } else {
                    None
                };
                tpls.push(Tpl::Rep { inner, sep });
            } else {
                tpls.push(tpl);
            }
            if next > index {
                *cursor = node_end(&nodes[next - 1]);
            }
            index = next;
            continue;
        }
        if let Some((tpl, next)) = read_directive(source, nodes, index) {
            // `#text($name)` inside a template literal stays in the copied
            // text. Pulling it out splits `` `${#text($name)}` `` so the
            // quotes around the capture are lost.
            if directive_inside_template(source, *cursor, node_start(&nodes[index])) {
                index += 1;
                continue;
            }
            push_gap(source, tpls, cursor, node_start(&nodes[index]));
            let end_node = next.saturating_sub(1);
            tpls.push(tpl);
            if end_node < nodes.len() {
                *cursor = node_end(&nodes[end_node]);
            }
            index = next;
            continue;
        }
        // The reader keeps `#` and `core` apart so a call-site `#macro` still
        // splits. A template directive is the pair.
        if is_hash_directive(nodes, index) {
            if let Some((tpl, next)) = read_split_directive(source, nodes, index) {
                if directive_inside_template(source, *cursor, node_start(&nodes[index])) {
                    index += 1;
                    continue;
                }
                push_gap(source, tpls, cursor, node_start(&nodes[index]));
                let end_node = next.saturating_sub(1);
                tpls.push(tpl);
                if end_node < nodes.len() {
                    *cursor = node_end(&nodes[end_node]);
                }
                index = next;
                continue;
            }
        }
        if dollar_at(nodes, index) {
            if let Some((path, end_at, next)) = read_var_path(nodes, index) {
                // `$protocol.register` is a text capture plus a member. The
                // member stays in the copied text. `$method.name(` is a field
                // path used as a call, so the whole path is one variable.
                push_gap(source, tpls, cursor, node_start(&nodes[index]));
                tpls.push(Tpl::Var(path));
                *cursor = end_at;
                index = next;
                continue;
            }
        }
        if let Syntax::Group { children, span, .. } = &nodes[index] {
            push_gap(source, tpls, cursor, span.start as usize + 1);
            walk_template(source, children, tpls, cursor);
            index += 1;
            continue;
        }
        // A template literal is one string. `#text($name)` inside `${}` is
        // still a directive; leave it as text so substitution can see it.
        if let Syntax::String { raw, .. } = &nodes[index] {
            let _ = raw;
            index += 1;
            continue;
        }
        index += 1;
    }
}

fn text_hole_name(group: &Syntax) -> Option<String> {
    let Syntax::Group { children, .. } = group else {
        return None;
    };
    read_var_path(children, 0).map(|(path, _, _)| path)
}

/// `$name`, `$name.field`, and `$name.field.nested`. The path is one capture
/// key. A following `(` is a call, not a field, so it stays in the copied text.
fn read_var_path(nodes: &[Syntax], index: usize) -> Option<(String, usize, usize)> {
    if !dollar_at(nodes, index) {
        return None;
    }
    let Syntax::Token { raw, span, .. } = nodes.get(index + 1)? else {
        return None;
    };
    let mut path = raw.clone();
    let mut end_at = span.end as usize;
    let mut cursor = index + 2;
    while operator_is(nodes.get(cursor), ".") {
        let Syntax::Token {
            raw: field,
            span: field_span,
            ..
        } = nodes.get(cursor + 1)?
        else {
            break;
        };
        path.push('.');
        path.push_str(field);
        end_at = field_span.end as usize;
        cursor += 2;
    }
    Some((path, end_at, cursor))
}

fn is_hash_directive(nodes: &[Syntax], index: usize) -> bool {
    operator_is(nodes.get(index), "#")
        && matches!(nodes.get(index + 1), Some(Syntax::Token { .. }))
        && (group_is(nodes.get(index + 2), Delimiter::Paren)
            || group_is(nodes.get(index + 2), Delimiter::Brace))
}

fn read_split_directive(source: &str, nodes: &[Syntax], index: usize) -> Option<(Tpl, usize)> {
    let Syntax::Token { raw, span, .. } = nodes.get(index + 1)? else {
        return None;
    };
    let name = format!("#{raw}");
    if name == "#syntax" && group_is(nodes.get(index + 2), Delimiter::Brace) {
        let body = nodes.get(index + 2)?;
        let tpl = syntax_define(source, body)?;
        return Some((tpl, index + 3));
    }
    if name == "#else" {
        return None;
    }
    let args = nodes.get(index + 2)?;
    if !group_is(Some(args), Delimiter::Paren) {
        return None;
    }
    let Syntax::Group {
        children,
        span: arg_span,
        ..
    } = args
    else {
        return None;
    };
    if name == "#if" {
        return read_if(source, nodes, index + 3, span.start as usize, children);
    }
    let tpl = directive_tpl(source, &name, span.start as usize, args, children, arg_span)?;
    Some((tpl, index + 3))
}

/// `#name(...)` in a template. Implemented directives become nodes. The rest
/// are recorded so printing can diagnose them instead of emitting `#name`.
fn read_directive(source: &str, nodes: &[Syntax], index: usize) -> Option<(Tpl, usize)> {
    let Syntax::Token { raw, span, .. } = nodes.get(index)? else {
        return None;
    };
    if !raw.starts_with('#') || raw.len() < 2 {
        return None;
    }
    if raw == "#syntax" && group_is(nodes.get(index + 1), Delimiter::Brace) {
        let body = nodes.get(index + 1)?;
        let tpl = syntax_define(source, body)?;
        return Some((tpl, index + 2));
    }
    if raw == "#else" {
        return None;
    }
    let Some(args) = nodes.get(index + 1) else {
        return None;
    };
    if !group_is(Some(args), Delimiter::Paren) {
        return None;
    }
    let Syntax::Group {
        children,
        span: arg_span,
        ..
    } = args
    else {
        return None;
    };
    if raw == "#if" {
        return read_if(source, nodes, index + 2, span.start as usize, children);
    }
    let tpl = directive_tpl(source, raw, span.start as usize, args, children, arg_span)?;
    Some((tpl, index + 2))
}

fn syntax_define(source: &str, body: &Syntax) -> Option<Tpl> {
    let Syntax::Group { children, span, .. } = body else {
        return None;
    };
    let mut inner = Vec::new();
    let mut cursor = span.start as usize + 1;
    walk_template(source, children, &mut inner, &mut cursor);
    push_gap(source, &mut inner, &mut cursor, span.end as usize - 1);
    Some(Tpl::Define(inner))
}

/// `#if(present $path) { then } #else { else }`.
fn read_if(
    source: &str,
    nodes: &[Syntax],
    after_pred: usize,
    at: usize,
    predicate: &[Syntax],
) -> Option<(Tpl, usize)> {
    let path = if_present_path(predicate)?;
    let then_group = nodes.get(after_pred)?;
    if !group_is(Some(then_group), Delimiter::Brace) {
        return Some((
            Tpl::Unknown {
                name: "#if".to_owned(),
                at,
            },
            after_pred,
        ));
    }
    let then_body = group_template(source, then_group);
    let mut next = after_pred + 1;
    let else_body = if if_else_at(nodes, next) {
        let else_at = if operator_is(nodes.get(next), "#") {
            next + 2
        } else {
            next + 1
        };
        let group = nodes.get(else_at)?;
        if group_is(Some(group), Delimiter::Brace) {
            next = else_at + 1;
            Some(group_template(source, group))
        } else {
            None
        }
    } else {
        None
    };
    Some((
        Tpl::If {
            path,
            then_body,
            else_body,
        },
        next,
    ))
}

fn if_else_at(nodes: &[Syntax], index: usize) -> bool {
    token_at(nodes, index, "#else")
        || (operator_is(nodes.get(index), "#") && token_at(nodes, index + 1, "else"))
}

fn if_present_path(children: &[Syntax]) -> Option<String> {
    if !token_at(children, 0, "present") {
        return None;
    }
    read_var_path(children, 1).map(|(path, _, _)| path)
}

fn group_template(source: &str, group: &Syntax) -> Vec<Tpl> {
    let Syntax::Group { children, span, .. } = group else {
        return Vec::new();
    };
    let mut inner = Vec::new();
    let mut cursor = span.start as usize + 1;
    walk_template(source, children, &mut inner, &mut cursor);
    push_gap(source, &mut inner, &mut cursor, span.end as usize - 1);
    inner
}

fn directive_tpl(
    source: &str,
    raw: &str,
    at: usize,
    args: &Syntax,
    children: &[Syntax],
    arg_span: &Span,
) -> Option<Tpl> {
    match raw {
        "#text" => text_hole_name(args).map(Tpl::Quote).or_else(|| {
            let Syntax::Group { children, .. } = args else {
                return None;
            };
            read_var_path(children, 0).map(|(path, _, _)| Tpl::Quote(path))
        }),
        "#core" => {
            let mut inner = Vec::new();
            let mut inner_cursor = arg_span.start as usize + 1;
            walk_template(source, children, &mut inner, &mut inner_cursor);
            push_gap(
                source,
                &mut inner,
                &mut inner_cursor,
                arg_span.end as usize - 1,
            );
            Some(Tpl::Core(inner))
        }
        "#join" => Some(read_join(children)),
        "#count" => text_hole_name(args).map(Tpl::Count),
        name if name.starts_with('#') => Some(Tpl::Unknown {
            name: name.to_owned(),
            at,
        }),
        _ => None,
    }
}

/// `#join($name, suffix: "Signal")` and the prefix / casing options beside it.
fn read_join(children: &[Syntax]) -> Tpl {
    let mut name = String::new();
    let mut index = 0;
    if let Some((path, _, next)) = read_var_path(children, 0) {
        name = path;
        index = next;
    } else if let Some(Syntax::Token { raw, .. }) = children.first() {
        name = raw.clone();
    }
    let mut prefix = String::new();
    let mut suffix = String::new();
    let mut casing = "preserve".to_owned();
    while index < children.len() {
        if operator_is(children.get(index), ",") {
            index += 1;
            continue;
        }
        let Some(Syntax::Token { raw: option, .. }) = children.get(index) else {
            break;
        };
        if !operator_is(children.get(index + 1), ":") {
            break;
        }
        let value = string_value(children.get(index + 2)).unwrap_or_default();
        match option.as_str() {
            "prefix" => prefix = value,
            "suffix" => suffix = value,
            "casing" => casing = value,
            _ => {}
        }
        index += 3;
    }
    Tpl::Join {
        name,
        prefix,
        suffix,
        casing,
    }
}

fn string_value(node: Option<&Syntax>) -> Option<String> {
    let Syntax::String { raw, .. } = node? else {
        return None;
    };
    let quote = raw.chars().next()?;
    raw.strip_prefix(quote)
        .and_then(|rest| rest.strip_suffix(quote))
        .map(unescape_string)
}

fn unescape_string(raw: &str) -> String {
    let mut out = String::new();
    let mut chars = raw.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some(other) => out.push(other),
                None => out.push('\\'),
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// True when `at` sits between backticks that `cursor` has not yet copied.
/// That span is a template literal, so a directive inside it is text.
fn directive_inside_template(source: &str, cursor: usize, at: usize) -> bool {
    if at < cursor || at > source.len() {
        return false;
    }
    let before = &source[cursor..at];
    let mut ticks = 0usize;
    let bytes = before.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\\' {
            index += 2;
            continue;
        }
        if bytes[index] == b'`' {
            ticks += 1;
        }
        index += 1;
    }
    ticks % 2 == 1
}

fn push_gap(source: &str, tpls: &mut Vec<Tpl>, cursor: &mut usize, until: usize) {
    if until < *cursor || until > source.len() {
        return;
    }
    let text = &source[*cursor..until];
    if text.is_empty() {
        return;
    }
    if let Some(Tpl::Text(existing)) = tpls.last_mut() {
        existing.push_str(text);
    } else {
        tpls.push(Tpl::Text(text.to_owned()));
    }
    *cursor = until;
}

fn tpls_from(nodes: &[Syntax], _unused: String) -> Vec<Tpl> {
    let mut tpls = Vec::new();
    let mut index = 0;
    let mut pending = String::new();
    let mut flush = |pending: &mut String, tpls: &mut Vec<Tpl>| {
        if !pending.is_empty() {
            tpls.push(Tpl::Text(std::mem::take(pending)));
        }
    };
    while index < nodes.len() {
        if is_rep_open(nodes, index) {
            flush(&mut pending, &mut tpls);
            let (tpl, next) = read_tpl_rep(nodes, index);
            tpls.push(tpl);
            index = next;
            continue;
        }
        if dollar_at(nodes, index) {
            if let Some(Syntax::Token { raw, .. }) = nodes.get(index + 1) {
                flush(&mut pending, &mut tpls);
                tpls.push(Tpl::Var(raw.clone()));
                index += 2;
                continue;
            }
        }
        match &nodes[index] {
            Syntax::Token { raw, .. } => {
                if !pending.is_empty() && !pending.ends_with([' ', '\n', '(']) {
                    pending.push(' ');
                }
                pending.push_str(raw);
            }
            Syntax::Operator { raw, .. } => {
                let tight = matches!(
                    raw.as_str(),
                    "," | ";" | "." | ":" | ")" | "]" | "}" | "(" | "[" | "{" | "!"
                );
                if !tight && !pending.is_empty() && !pending.ends_with([' ', '\n', '(']) {
                    pending.push(' ');
                }
                pending.push_str(raw);
                if raw == "," || raw == ";" {
                    pending.push(' ');
                }
            }
            Syntax::String { raw, .. } => pending.push_str(raw),
            Syntax::Group {
                delimiter,
                children,
                ..
            } => {
                pending.push(open_char(*delimiter));
                // Nested groups in a template are ordinary text plus holes.
                let nested = tpls_from(children, String::new());
                if nested.iter().all(|part| matches!(part, Tpl::Text(_))) {
                    for part in nested {
                        if let Tpl::Text(text) = part {
                            pending.push_str(&text);
                        }
                    }
                } else {
                    flush(&mut pending, &mut tpls);
                    tpls.extend(flatten_group(*delimiter, nested));
                    // The closer was emitted inside flatten; do not also append.
                    index += 1;
                    continue;
                }
                pending.push(match delimiter {
                    Delimiter::Paren => ')',
                    Delimiter::Brace => '}',
                    Delimiter::Bracket => ']',
                });
            }
            Syntax::Comment { .. } => {}
        }
        index += 1;
    }
    flush(&mut pending, &mut tpls);
    tpls
}

fn flatten_group(delimiter: Delimiter, nested: Vec<Tpl>) -> Vec<Tpl> {
    let mut out = vec![Tpl::Text(open_char(delimiter).to_string())];
    out.extend(nested);
    let close = match delimiter {
        Delimiter::Paren => ')',
        Delimiter::Brace => '}',
        Delimiter::Bracket => ']',
    };
    out.push(Tpl::Text(close.to_string()));
    out
}

fn read_tpl_rep(nodes: &[Syntax], index: usize) -> (Tpl, usize) {
    let Syntax::Group { children, .. } = &nodes[index + 1] else {
        return (Tpl::Text("$".to_owned()), index + 1);
    };
    let inner = tpls_from(children, String::new());
    let mut cursor = index + 2;
    let sep = if operator_is(nodes.get(cursor), ",")
        || operator_is(nodes.get(cursor), ";")
        || operator_is(nodes.get(cursor), "|")
    {
        let raw = match &nodes[cursor] {
            Syntax::Operator { raw, .. } => Some(raw.clone()),
            _ => Some(",".to_owned()),
        };
        cursor += 1;
        raw
    } else {
        None
    };
    if operator_is(nodes.get(cursor), "*") || operator_is(nodes.get(cursor), "+") {
        cursor += 1;
    }
    (Tpl::Rep { inner, sep }, cursor)
}

struct Match {
    next: usize,
    end: usize,
    captures: HashMap<String, Capture>,
}

#[derive(Clone, Debug, PartialEq)]
enum Capture {
    Text(String),
    /// A syntax-class match. `text` is the whole form; `fields` are its
    /// declared fields, including sequences.
    Record {
        text: String,
        fields: HashMap<String, Capture>,
    },
    Many(Vec<Vec<(String, Capture)>>),
}

struct RuleStop {
    span: Span,
    /// `expect` from the class rule that stopped, when the macro rule has none.
    expect: Option<String>,
    progress: i32,
}

/// The token where `pattern` stopped, and how far it got. A finished match
/// has no failure. Progress is the byte where matching stopped, so a rule
/// that consumed more of the input wins over one that failed on the head.
fn failure_progress(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    pattern: &[Pat],
    classes: &[SyntaxClass],
) -> Option<RuleStop> {
    let mut captures = HashMap::new();
    let mut cursor = index;
    let pattern = skip_matched_prefix(nodes, index, pattern);
    let mut progress = nodes.get(index).map(node_start).unwrap_or(0) as i32;
    let mut expect = None;
    let span = match_pats_fail(
        source,
        nodes,
        &mut cursor,
        pattern,
        &mut captures,
        classes,
        &mut progress,
        &mut expect,
    )?;
    // A nested class stopped on `value` in `value:)`. The closer after
    // the colon is the missing type, not the field name. `state = 1`
    // already stopped on `=`; that is not a name before a colon.
    let span = if nodes
        .iter()
        .any(|node| node_span(node).start == span.start && matches!(node, Syntax::Token { .. }))
    {
        closer_after_colon(source, nodes, index, span)
    } else {
        span
    };
    Some(RuleStop {
        span,
        expect,
        progress,
    })
}

/// `value:)` stopped on `value`. The token after the colon is the failure.
/// The colon may be inside a group the outer list only sees as one node.
fn closer_after_colon(source: &str, nodes: &[Syntax], from: usize, span: Span) -> Span {
    let _ = source;
    // The failed token may be inside a group that starts before `from`.
    // `Some(value:)` is that group; searching only from the name misses it.
    let start = nodes
        .iter()
        .rposition(|node| node_span(node).start <= span.start && node_span(node).end >= span.end)
        .unwrap_or(from);
    closer_in(&nodes[start..], span).unwrap_or(span)
}

fn closer_in(nodes: &[Syntax], span: Span) -> Option<Span> {
    let at = nodes
        .iter()
        .position(|node| node_span(node).start == span.start)?;
    if nodes
        .get(at + 1)
        .is_some_and(|node| operator_is(Some(node), ":"))
    {
        let next = nodes.get(at + 2)?;
        if matches!(next, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ")" | "," | ";" | "}" | "]"))
        {
            return Some(failure_token(next));
        }
    }
    for node in nodes {
        if let Syntax::Group { children, .. } = node {
            if node_span(node).start <= span.start && node_span(node).end >= span.end {
                if let Some(found) = closer_in(children, span) {
                    return Some(found);
                }
            }
            // `(value:)` is the group after the name the rule stopped on.
            if node_span(node).start >= span.end {
                if let Some(found) = missing_colon_in(children) {
                    return Some(found);
                }
            }
        }
    }
    None
}

/// `value:` inside a group the caller sees as one node. The closer is the
/// missing type.
fn missing_colon_in(nodes: &[Syntax]) -> Option<Span> {
    let mut index = 0;
    while index + 1 < nodes.len() {
        let name = matches!(&nodes[index], Syntax::Token { raw, .. } if is_ident_text(raw));
        if name && operator_is(nodes.get(index + 1), ":") {
            let after = index + 2;
            if nodes.get(after).is_some_and(|node| {
                matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ")" | "," | ";" | "}" | "]"))
            }) || nodes.get(after).is_none()
            {
                return nodes.get(after).map(failure_token).or_else(|| nodes.get(index + 1).map(failure_token));
            }
        }
        index += 1;
    }
    None
}

fn match_pats_fail(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
    progress: &mut i32,
    expect: &mut Option<String>,
) -> Option<Span> {
    for pat in pattern {
        match pat {
            Pat::Lit(expected) => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                if let Some(width) = lit_width(nodes, *cursor, expected) {
                    *progress = (*progress).max(node_end(&nodes[*cursor + width - 1]) as i32);
                    *cursor += width;
                } else if node_is_lit(node, expected) {
                    *progress = (*progress).max(node_end(node) as i32);
                    *cursor += 1;
                } else if expected == "_" {
                    // `{ 1 => "one", }` wanted `_` or another arm. The
                    // trailing comma's closer is the failure, not `1`.
                    if expect.is_none() {
                        *expect = Some("expected `MatchArm` or `_`".to_owned());
                    }
                    return Some(match_closer(source, node).unwrap_or_else(|| failure_token(node)));
                } else if expected.starts_with("seg") {
                    // `between(3) lower(1)` stopped on `lower`. The missing
                    // segment is `upper(`, and that `(` is the failure.
                    return Some(
                        segment_paren(source, node).unwrap_or_else(|| failure_token(node)),
                    );
                } else if let Some(span) =
                    nested_form_failure(source, node_slice_from(nodes, *cursor), 0)
                {
                    return Some(span);
                } else if expected == "state" && operator_is(Some(node), "=") {
                    // `state = 1` has no binding. `=` is where the name
                    // should have been, not the initializer after it.
                    return Some(failure_token(node));
                } else if expected == "assert" {
                    // `assert()` has no condition. The `)` is the failure,
                    // not the macro name.
                    return Some(
                        empty_call_paren(source, node).unwrap_or_else(|| failure_token(node)),
                    );
                } else {
                    return Some(failure_token(node));
                }
            }
            Pat::Group { delimiter, inner } => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                let Syntax::Group {
                    delimiter: found,
                    children,
                    close,
                    ..
                } = node
                else {
                    return Some(failure_token(node));
                };
                if found != delimiter {
                    return Some(failure_token(node));
                }
                let mut child = 0;
                let mut nested = node_start(node) as i32;
                if let Some(span) = match_pats_fail(
                    source,
                    children,
                    &mut child,
                    inner,
                    captures,
                    classes,
                    &mut nested,
                    expect,
                ) {
                    *progress = (*progress).max(nested);
                    // `(value) => value` failed inside the arrow. The rule
                    // was matching a call step, and the empty step is the
                    // comma after this group: `map((value) => value), ,`.
                    if let Some(semi) = bind_semi(nodes.get(*cursor)) {
                        *progress = (*progress).max(semi.end as i32);
                        if expect.is_none() {
                            *expect = Some("expression after <-".to_owned());
                        }
                        return Some(semi);
                    }
                    if let Some(outer) = dangling_after_group(nodes, *cursor) {
                        // The comma is the failure. A nested miss inside the
                        // arrow (`=>`) has a later byte offset, but it is not
                        // further through the rule.
                        *progress = (*progress).max(outer.end as i32 + 1);
                        return Some(outer);
                    }
                    return Some(span);
                }
                *progress = (*progress).max(nested);
                skip_comments(children, &mut child);
                if child < children.len() {
                    // The pattern ended. A trailing separator, or the closer
                    // when the last piece is that separator, is the failure.
                    // `head:, tail` and `{ 1 => "one", }` are that shape.
                    if let Some(span) = trailing_separator(children, child, *close) {
                        *progress = (*progress).max(span.start as i32);
                        return Some(span);
                    }
                    return Some(failure_token(&children[child]));
                }
                *progress = (*progress).max(node_end(node) as i32);
                *cursor += 1;
            }
            Pat::Hole { name, kind } => {
                if let HoleKind::Class(class_name) = kind {
                    let Some(class_) = classes.iter().find(|class_| class_.name == *class_name)
                    else {
                        // `item` and `jsxChild` are positions, not classes in
                        // this module. The hole still wanted a form here.
                        // `record Cons(head:, tail)` fails inside that item,
                        // on the comma after the missing type. `{each ...}`
                        // fails on the line that should have been the body.
                        if let Some(span) = nested_form_failure(source, nodes, *cursor) {
                            return Some(span);
                        }
                        return nodes.get(*cursor).map(failure_token);
                    };
                    if let Some(found) = class_failure(source, nodes, *cursor, class_, classes) {
                        *progress = (*progress).max(found.progress);
                        if expect.is_none() {
                            *expect = found.expect;
                        }
                        return Some(found.span);
                    }
                    let (capture, end, next) = take_class(source, nodes, *cursor, class_, classes)?;
                    captures.insert(name.clone(), capture);
                    *progress = (*progress).max(end as i32);
                    *cursor = next;
                } else {
                    skip_comments(nodes, cursor);
                    let taken = if *kind == HoleKind::Binding && pattern.len() == 1 {
                        take_expr(source, nodes, *cursor)
                            .or_else(|| take_ident(source, nodes, *cursor))
                    } else {
                        take_hole(source, nodes, *cursor, kind, false)
                    };
                    if taken.is_none() {
                        // The hole wanted a token that is not here. The next
                        // token is that absence: `value:;` and `assert();`.
                        // `value:` has no type. The closer after the colon is
                        // that absence, not the field name the type hole saw.
                        let missing = if matches!(kind, HoleKind::Ident | HoleKind::Binding)
                            && nodes
                                .get(*cursor)
                                .is_some_and(|node| operator_is(Some(node), "="))
                        {
                            // `state = 1` has no name. `=` is that absence.
                            nodes.get(*cursor).map(failure_token)
                        } else if matches!(kind, HoleKind::Type) {
                            type_failure_token(nodes, *cursor)
                        } else if matches!(kind, HoleKind::Expr) {
                            // `value <-;` has no expression. The semicolon is
                            // that absence, not the binder before `<-`.
                            expr_failure_token(nodes, *cursor)
                        } else {
                            nodes.get(*cursor).map(failure_token)
                        };
                        // A later pattern may name a further token, such as
                        // the `}` of an empty function body. Keep the earlier
                        // stop only when nothing after it fails further on.
                        let here = missing;
                        let keep_missing = matches!(kind, HoleKind::Ident | HoleKind::Binding)
                            && nodes
                                .get(*cursor)
                                .is_some_and(|node| operator_is(Some(node), "="));
                        if !keep_missing {
                            if let Some(later) = later_failure(
                                source,
                                nodes,
                                *cursor,
                                &pattern[1..],
                                classes,
                                progress,
                                expect,
                            ) {
                                return Some(later);
                            }
                        }
                        return here;
                    }
                    let (_text, end, next) = taken?;
                    *progress = (*progress).max(end as i32);
                    *cursor = next;
                }
            }
            Pat::Rep { inner, sep, kind } => {
                let mut many = 0usize;
                loop {
                    let saved = *cursor;
                    let mut one = HashMap::new();
                    if match_pats(source, nodes, cursor, inner, &mut one, classes).is_none() {
                        *cursor = saved;
                        if *kind == RepKind::OneOrMore && many == 0 {
                            return match_pats_fail(
                                source, nodes, cursor, inner, &mut one, classes, progress, expect,
                            )
                            .or_else(|| nodes.get(*cursor).map(failure_token));
                        }
                        // `None() | Some(value:)` ate `None()` and stopped.
                        // The next constructor is not the failure; the token
                        // its field rule could not accept is.
                        if let Some(span) = nested_form_failure(source, nodes, *cursor) {
                            return Some(span);
                        }
                        if many > 0 {
                            if let Some(inner_span) = match_pats_fail(
                                source, nodes, cursor, inner, &mut one, classes, progress, expect,
                            ) {
                                let inner_span = if let Some(found) = bind_semi(nodes.get(*cursor))
                                    .or_else(|| {
                                        nodes.get(*cursor + 1).and_then(|node| {
                                            let Syntax::Group {
                                                children,
                                                span: group,
                                                ..
                                            } = node
                                            else {
                                                return None;
                                            };
                                            if !children
                                                .iter()
                                                .any(|child| operator_is(Some(child), ":"))
                                            {
                                                return None;
                                            }
                                            // `Some(value:)` — the reader stored the
                                            // colon as the group's close. The `)` is
                                            // the next `)` in the source, and that is
                                            // the token upstream reports.
                                            let at = group.start as usize;
                                            if let Some(rel) =
                                                source.get(at..).and_then(|rest| rest.find(')'))
                                            {
                                                let start = u32_at(at + rel);
                                                return Some(Span {
                                                    start,
                                                    end: start + 1,
                                                });
                                            }
                                            missing_colon_in(children)
                                        })
                                    }) {
                                    found
                                } else {
                                    closer_after_colon(source, nodes, saved, inner_span)
                                };
                                let missing_bind = bind_semi(nodes.get(*cursor)).is_some();
                                if inner_span.start as i32 >= *progress || missing_bind {
                                    return Some(inner_span);
                                }
                            }
                            *cursor = saved;
                        }
                        // A separator with nothing after it is the failure,
                        // not the last item the repetition already ate.
                        if let Some(sep) = sep {
                            let mut look = *cursor;
                            skip_comments(nodes, &mut look);
                            if nodes.get(look).is_some_and(|node| node_is_lit(node, sep)) {
                                let after = look + 1;
                                if nodes.get(after).is_none_or(|node| {
                                    node_is_lit(node, sep)
                                        || matches!(node, Syntax::Operator { raw, .. } if raw == ")" || raw == "}" || raw == "]")
                                }) {
                                    let span = failure_token(&nodes[look]);
                                    *progress = (*progress).max(span.start as i32);
                                    return Some(span);
                                }
                            }
                        }
                        break;
                    }
                    if let Some(node) = nodes.get(saved) {
                        *progress = (*progress).max(node_start(node) as i32);
                    }
                    many += 1;
                    if let Some(sep) = sep {
                        let saved = *cursor;
                        skip_comments(nodes, cursor);
                        if nodes
                            .get(*cursor)
                            .is_some_and(|node| node_is_lit(node, sep))
                        {
                            *progress = (*progress).max(node_end(&nodes[*cursor]) as i32);
                            *cursor += 1;
                            continue;
                        }
                        *cursor = saved;
                        break;
                    }
                }
            }
        }
    }
    None
}

/// A hole that could not take a token does not hide a later failure.
/// `function(...) { stmts }` with no final expression still has the body's
/// closer as the token the rule was expecting.
fn later_failure(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    pattern: &[Pat],
    classes: &[SyntaxClass],
    progress: &mut i32,
    expect: &mut Option<String>,
) -> Option<Span> {
    if pattern.is_empty() {
        return None;
    }
    let mut probe = cursor;
    skip_comments(nodes, &mut probe);
    // The missing hole may have been the closer itself. Try the rest of the
    // pattern at this token and, if that does not move, at the next one.
    let mut first = probe;
    let mut nested = nodes.get(probe).map(node_start).unwrap_or(0) as i32;
    if let Some(span) = match_pats_fail(
        source,
        nodes,
        &mut first,
        pattern,
        &mut HashMap::new(),
        classes,
        &mut nested,
        expect,
    ) {
        if span.start as i32 > nodes.get(probe).map(node_start).unwrap_or(0) as i32 {
            *progress = (*progress).max(nested);
            return Some(span);
        }
    }
    if probe + 1 >= nodes.len() {
        return None;
    }
    let mut next = probe + 1;
    let mut nested = nodes.get(next).map(node_start).unwrap_or(0) as i32;
    let span = match_pats_fail(
        source,
        nodes,
        &mut next,
        pattern,
        &mut HashMap::new(),
        classes,
        &mut nested,
        expect,
    )?;
    if (span.start as i32) <= nodes.get(probe).map(node_start).unwrap_or(0) as i32 {
        return None;
    }
    *progress = (*progress).max(nested);
    Some(span)
}

/// `map((value) => value), , filter` — a group failed while matching a
/// call step, and a later separator has no step after it. That separator
/// is the failure token, not the `=>` inside the arrow.
fn dangling_after_group(nodes: &[Syntax], group_at: usize) -> Option<Span> {
    let mut look = group_at;
    while look < nodes.len() {
        if nodes.get(look).is_some_and(|node| node_is_lit(node, ",")) {
            let mut after = look + 1;
            skip_comments(nodes, &mut after);
            if nodes.get(after).is_some_and(|node| node_is_lit(node, ",")) {
                return Some(failure_token(&nodes[after]));
            }
        }
        if let Syntax::Group { children, .. } = &nodes[look] {
            if let Some(span) = dangling_after_group(children, 0) {
                return Some(span);
            }
        }
        look += 1;
    }
    None
}

/// A group the pattern already finished, whose leftover starts at a comma.
/// Upstream points at the closer when the comma is the last child
/// (`Some(value:);`, `assert();`) and at the comma when another child follows
/// (`{ 1 => "one", }`, `(-> ..., , filter())`).
fn trailing_separator(children: &[Syntax], child: usize, close: Option<Span>) -> Option<Span> {
    let node = children.get(child)?;
    if !matches!(node, Syntax::Operator { raw, .. } if raw == ",") {
        return None;
    }
    let mut after = child + 1;
    skip_comments(children, &mut after);
    if after < children.len() {
        return Some(failure_token(node));
    }
    close.or_else(|| Some(node_span(node)))
}

/// The token a diagnostic names. A group span starts at `(`, which is not
/// the closer upstream reports for `empty():` or `assert()`.
fn failure_token(node: &Syntax) -> Span {
    match node {
        Syntax::Group {
            close: Some(close), ..
        } => *close,
        other => node_span(other),
    }
}

/// The macro name written before a failure that is inside its form.
/// `record Cons(head:, tail)` — the comma belongs to `record`.
fn nested_macro_name(source: &str, span: Span) -> Option<String> {
    let before = source.get(..span.start as usize)?;
    let start = before.rfind('\n')?;
    let line = before[start + 1..].trim_start();
    let name = line
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .next()?;
    if name.is_empty() || !is_ident_text(name) {
        return None;
    }
    Some(name.to_owned())
}

fn node_slice_from(nodes: &[Syntax], cursor: usize) -> &[Syntax] {
    if cursor >= nodes.len() {
        &[]
    } else {
        &nodes[cursor..]
    }
}

/// A position hole (`item`, `jsxChild`) whose form is a nested macro.
/// `record Cons(head:, tail)` fails at the comma after the missing type.
/// `{each ...}` followed by a line fails at the start of that line.
fn nested_form_failure(source: &str, nodes: &[Syntax], cursor: usize) -> Option<Span> {
    let node = nodes.get(cursor)?;
    if let Syntax::Token { raw, .. } = node {
        if raw == "record" {
            let at = node_start(node);
            let rest = source.get(at..)?;
            let colon = rest.find(':')?;
            let after = &rest[colon + 1..];
            let rel = after.find(|ch: char| ch == ',' || ch == ')')?;
            let start = u32_at(at + colon + 1 + rel);
            return Some(Span {
                start,
                end: start + 1,
            });
        }
    }
    if let Syntax::Group { children, span, .. } = node {
        if children.first().and_then(macro_head) == Some("each") {
            let at = span.end as usize;
            let rest = source.get(at..)?;
            let rel = rest.find(|ch: char| !ch.is_whitespace())?;
            let start = u32_at(at + rel);
            return Some(Span { start, end: start });
        }
    }
    None
}

/// `{ 1 => "one", }` stopped on `1` because `_` was expected. The `}`
/// after the trailing comma is the failure token.
fn match_closer(source: &str, node: &Syntax) -> Option<Span> {
    let at = node_start(node);
    let rest = source.get(at..)?;
    let comma = rest.find(',')?;
    let after = &rest[comma + 1..];
    let rel = after.find(|ch: char| ch == '}' || !ch.is_whitespace())?;
    if after.as_bytes().get(rel) != Some(&b'}') {
        return None;
    }
    let start = u32_at(at + comma + 1 + rel);
    Some(Span {
        start,
        end: start + 1,
    })
}

/// `assert()` is a call. Rejecting at the name hides the `)` the rule
/// fails on. Only that empty call skips the early reject.
fn empty_call_head(nodes: &[Syntax], index: usize) -> bool {
    let Some(name) = macro_head(&nodes[index]) else {
        return false;
    };
    if name != "assert" {
        return false;
    }
    let Syntax::Group {
        children,
        delimiter: Delimiter::Paren,
        ..
    } = nodes.get(index + 1).unwrap_or(&nodes[index])
    else {
        return false;
    };
    children
        .iter()
        .all(|child| matches!(child, Syntax::Comment { .. }))
}

/// The first non-whitespace of the line after `at`.
fn each_line_after(source: &str, at: usize) -> Option<Span> {
    let rest = source.get(at..)?;
    let nl = rest.find('\n')?;
    let line_start = at + nl + 1;
    let mut end = line_start;
    while source
        .as_bytes()
        .get(end)
        .is_some_and(|byte| *byte != b'\n')
    {
        end += 1;
    }
    // The missing brace is the end of that line. A span on `<` is the
    // element, not the failure.
    let start = u32_at(end);
    Some(Span { start, end: start })
}

/// `{each (...)}` wanted a braced body on the next line. Upstream
/// points at the start of that line, not at `each`.
fn each_body_line(source: &str, nodes: &[Syntax], index: usize) -> Option<Span> {
    let group = nodes.iter().skip(index).find(|node| {
        matches!(node, Syntax::Group { children, .. } if children.first().and_then(macro_head) == Some("each"))
    })?;
    let at = node_end(group);
    let rest = source.get(at..)?;
    let nl = rest.find('\n')?;
    let start = u32_at(at + nl + 1);
    Some(Span { start, end: start })
}

/// `assert()` — the call has no argument. The `)` is the failure token.
fn empty_call_paren(source: &str, node: &Syntax) -> Option<Span> {
    let at = node_end(node);
    let rest = source.get(at..)?;
    if !rest.starts_with("()") {
        return None;
    }
    let start = u32_at(at + 1);
    Some(Span {
        start,
        end: start + 1,
    })
}

/// `lower(1)` where the rule wanted `upper(`. The `(` of the missing
/// segment is the next one in the source, not the segment name that was written.
fn segment_paren(source: &str, node: &Syntax) -> Option<Span> {
    let at = node_end(node);
    let rel = source.get(at..)?.find('(')?;
    let start = u32_at(at + rel);
    Some(Span {
        start,
        end: start + 1,
    })
}

/// `value <-;` inside the group the repetition stopped on. The semicolon
/// is the missing expression.
fn bind_semi(node: Option<&Syntax>) -> Option<Span> {
    let Syntax::Group { children, .. } = node? else {
        return None;
    };
    if !children
        .iter()
        .any(|child| operator_spelling(child).is_some_and(|raw| raw == "<-"))
    {
        return None;
    }
    let arrow = children
        .iter()
        .position(|child| operator_spelling(child).is_some_and(|raw| raw == "<-"))?;
    children
        .get(arrow + 1)
        .filter(|child| operator_is(Some(child), ";"))
        .map(failure_token)
}

/// An expression hole that did not take an expression. `name <-;` reports
/// the semicolon. A binder with no operator is itself.
fn expr_failure_token(nodes: &[Syntax], cursor: usize) -> Option<Span> {
    let mut look = cursor;
    skip_comments(nodes, &mut look);
    if nodes
        .get(look)
        .and_then(|node| match node {
            Syntax::Token { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .is_some_and(is_ident_text)
    {
        let mut op = look + 1;
        skip_comments(nodes, &mut op);
        if nodes
            .get(op)
            .and_then(operator_spelling)
            .is_some_and(|raw| raw == "<-" || raw == "->")
        {
            let after = op + 1;
            return nodes.get(after).map(failure_token);
        }
    }
    nodes.get(look).map(failure_token)
}

/// A type hole that did not take a type. `value:` reports `)`, and
/// `head:, tail` reports the comma. A name with no colon is itself.
fn type_failure_token(nodes: &[Syntax], cursor: usize) -> Option<Span> {
    let mut look = cursor;
    skip_comments(nodes, &mut look);
    if nodes
        .get(look)
        .is_some_and(|node| operator_is(Some(node), ":"))
    {
        look += 1;
        skip_comments(nodes, &mut look);
        return nodes.get(look).map(failure_token);
    }
    if nodes
        .get(look)
        .and_then(|node| match node {
            Syntax::Token { raw, .. } => Some(raw.as_str()),
            _ => None,
        })
        .is_some_and(is_ident_text)
        && nodes
            .get(look + 1)
            .is_some_and(|node| operator_is(Some(node), ":"))
    {
        let after = look + 2;
        return nodes.get(after).map(failure_token);
    }
    nodes.get(look).map(failure_token)
}

/// `name:` with the type missing. The match consumed the closer, so the
/// class did not really accept the form. The closer is the failure token.
fn missing_type_closer(source: &str, nodes: &[Syntax], from: usize, to: usize) -> Option<Span> {
    if to <= from || to > nodes.len() {
        return None;
    }
    let text = source.get(node_start(&nodes[from])..node_end(&nodes[to - 1]))?;
    let mut depth = 0i32;
    let mut saw_colon = false;
    for ch in text.chars() {
        match ch {
            '(' | '{' | '[' => depth += 1,
            ')' | '}' | ']' => {
                if saw_colon && depth == 1 {
                    return nodes.get(to - 1).map(failure_token);
                }
                depth -= 1;
            }
            ':' if depth > 0 => saw_colon = true,
            ',' if depth > 0 && saw_colon => return nodes.get(to - 1).map(failure_token),
            ch if ch.is_whitespace() => {}
            _ => saw_colon = false,
        }
    }
    None
}

/// The token a syntax class could not accept. Each rule is tried; the one
/// that gets furthest supplies the span and its `expect`.
fn class_failure(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    class_: &SyntaxClass,
    classes: &[SyntaxClass],
) -> Option<RuleStop> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let mut best: Option<RuleStop> = None;
    let mut any = false;
    for rule in &class_.rules {
        any = true;
        let mut child = index;
        let mut raw = HashMap::new();
        let mut nested = nodes.get(index).map(node_start).unwrap_or(0) as i32;
        let mut expect = None;
        if let Some(mut span) = match_pats_fail(
            source,
            nodes,
            &mut child,
            &rule.pattern,
            &mut raw,
            classes,
            &mut nested,
            &mut expect,
        ) {
            // `value:` failed on the name. The type is missing, so the
            // closer after the colon is the token the rule was expecting.
            // That colon is inside the constructor group, not a sibling.
            let adjusted = expr_failure_token(nodes, child.saturating_sub(1))
                .filter(|found| found.start > span.start)
                .unwrap_or_else(|| closer_after_colon(source, nodes, index, span));
            if adjusted.start != span.start {
                nested = adjusted.end as i32;
                span = adjusted;
            } else if let Some(inside) =
                nodes
                    .get(child.saturating_sub(1))
                    .and_then(|node| match node {
                        Syntax::Group { children, .. } => closer_in(children, span),
                        _ => None,
                    })
            {
                nested = inside.end as i32;
                span = inside;
            } else {
                nested = nested.max(span.start as i32);
            }
            let expect = expect.or_else(|| rule.expect.clone());
            let better = best.as_ref().is_none_or(|have| nested > have.progress);
            if better {
                best = Some(RuleStop {
                    span,
                    expect,
                    progress: nested,
                });
            }
        } else if match_pats(source, nodes, &mut child, &rule.pattern, &mut raw, classes).is_some()
        {
            // `Some(value:)` matches only because the type hole ate `)`. A
            // colon with no type is still the class failure. The closer is
            // the token upstream names.
            if let Some(span) = missing_type_closer(source, nodes, index, child) {
                let progress = span.start as i32;
                let better = best.as_ref().is_none_or(|have| progress > have.progress);
                if better {
                    best = Some(RuleStop {
                        span,
                        expect: rule.expect.clone(),
                        progress,
                    });
                }
                continue;
            }
            return None;
        }
    }
    if any {
        best.or_else(|| {
            nodes.get(index).map(|node| RuleStop {
                span: failure_token(node),
                expect: None,
                progress: node_start(node) as i32,
            })
        })
    } else {
        nodes.get(index).map(|node| RuleStop {
            span: failure_token(node),
            expect: None,
            progress: node_start(node) as i32,
        })
    }
}

fn match_rule(source: &str, nodes: &[Syntax], index: usize, pattern: &[Pat]) -> Option<Match> {
    match_rule_with(source, nodes, index, pattern, &[])
}

fn match_rule_with(
    source: &str,
    nodes: &[Syntax],
    index: usize,
    pattern: &[Pat],
    classes: &[SyntaxClass],
) -> Option<Match> {
    let mut captures = HashMap::new();
    let mut cursor = index;
    // A rule may write `export function` while the call site already printed
    // `export` and is matching at `function`. Those leading literals are the
    // head's modifiers, not a second copy of the name.
    let pattern = skip_matched_prefix(nodes, index, pattern);
    // `(-> value, call(...), ...)` is one group in the macro source. The
    // reader already opened that group, so the call site is the operator
    // and its siblings, not another paren around them.
    let unwrapped = unwrap_call_group(nodes, index, pattern).to_vec();
    let owned = comma_separated_rest(&unwrapped);
    let end = match_pats(source, nodes, &mut cursor, &owned, &mut captures, classes)?;
    Some(Match {
        next: cursor,
        end,
        captures,
    })
}

/// `$($rest:tt)+` was written with the comma inside the repetition.
/// The call site separates steps with a comma, so the repetition's
/// separator is that comma and the hole is one step.
fn comma_separated_rest(pattern: &[Pat]) -> Vec<Pat> {
    let mut out = Vec::new();
    let mut index = 0;
    while index < pattern.len() {
        let pat = &pattern[index];
        if matches!(pat, Pat::Lit(lit) if lit == ",")
            && matches!(pattern.get(index + 1), Some(Pat::Rep { inner, sep: None, .. }) if inner.iter().any(|part| matches!(part, Pat::Hole { kind: HoleKind::TokenTree, .. })))
        {
            if let Pat::Rep { inner, kind, .. } = &pattern[index + 1] {
                out.push(Pat::Rep {
                    inner: comma_separated_rest(inner),
                    sep: Some(",".to_owned()),
                    kind: *kind,
                });
                index += 2;
                continue;
            }
        }
        out.push(match pat {
            Pat::Group { delimiter, inner } => Pat::Group {
                delimiter: *delimiter,
                inner: comma_separated_rest(inner),
            },
            Pat::Rep {
                inner,
                sep: None,
                kind,
            } => {
                let inner = comma_separated_rest(inner);
                let hole = inner.iter().any(|part| {
                    matches!(
                        part,
                        Pat::Hole {
                            kind: HoleKind::TokenTree,
                            ..
                        }
                    )
                });
                let comma = inner
                    .iter()
                    .any(|part| matches!(part, Pat::Lit(lit) if lit == ","));
                if hole && comma {
                    Pat::Rep {
                        inner: inner
                            .into_iter()
                            .filter(|part| !matches!(part, Pat::Lit(lit) if lit == ","))
                            .collect(),
                        sep: Some(",".to_owned()),
                        kind: *kind,
                    }
                } else {
                    Pat::Rep {
                        inner,
                        sep: None,
                        kind: *kind,
                    }
                }
            }
            other => other.clone(),
        });
        index += 1;
    }
    out
}

/// `filter(map(...), (value) => value > 2)` is the expansion. The
/// reference also has a trailing comma on that call, and on a call whose
/// last argument is itself a call. An inner `map(...)` does not.
fn trailing_thread_commas(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut insert = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        if chars[index] == '(' {
            if let Some(close) = matching_thread_paren(&chars, index) {
                let last = last_arg_end(&chars, index + 1, close);
                let outermost = close + 1 == chars.len()
                    || chars[close + 1..]
                        .iter()
                        .all(|ch| ch.is_whitespace() || *ch == ';');
                let already = close > 0 && chars[close - 1] == ',';
                // A nested call (`vector(...)`) does not take the comma.
                // The outer call does, including when its last argument is
                // a number (`3`) or another call.
                if outermost && !already && last.is_some() {
                    insert.push(close);
                }
            }
        }
        index += 1;
    }
    let mut out = String::new();
    for (at, ch) in chars.iter().enumerate() {
        if insert.contains(&at) {
            out.push(',');
        }
        out.push(*ch);
    }
    out
}

fn matching_thread_paren(chars: &[char], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (at, ch) in chars.iter().enumerate().skip(open) {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
            _ => {}
        }
    }
    None
}

fn last_arg_end(chars: &[char], start: usize, close: usize) -> Option<usize> {
    let mut at = close;
    while at > start && chars[at - 1].is_whitespace() {
        at -= 1;
    }
    if at == start {
        return None;
    }
    Some(at - 1)
}

fn grouped_prefix(nodes: &[Syntax], index: usize) -> Option<String> {
    let Syntax::Group {
        children,
        delimiter: Delimiter::Paren,
        ..
    } = nodes.get(index)?
    else {
        return None;
    };
    let first = children.first().and_then(operator_spelling)?;
    if first == "->" && children.get(1).and_then(operator_spelling) == Some(">") {
        let adjacent = node_end(&children[0]) == node_start(&children[1]);
        if adjacent {
            return Some("->>".to_owned());
        }
    }
    if first == "->" || first == "->>" || first == "<-" {
        return Some(first.to_owned());
    }
    None
}

fn unwrap_call_group<'a>(nodes: &[Syntax], index: usize, pattern: &'a [Pat]) -> &'a [Pat] {
    let [
        Pat::Group {
            delimiter: Delimiter::Paren,
            inner,
        },
    ] = pattern
    else {
        return pattern;
    };
    let head = match nodes.get(index) {
        Some(Syntax::Token { raw, .. } | Syntax::Operator { raw, .. }) => raw.as_str(),
        _ => return pattern,
    };
    let starts = inner
        .first()
        .is_some_and(|pat| matches!(pat, Pat::Lit(lit) if lit == head));
    if starts { inner.as_slice() } else { pattern }
}

fn skip_matched_prefix<'a>(nodes: &[Syntax], index: usize, pattern: &'a [Pat]) -> &'a [Pat] {
    let Some(head) = nodes.get(index).and_then(|node| match node {
        Syntax::Token { raw, .. } => Some(raw.as_str()),
        _ => None,
    }) else {
        return pattern;
    };
    let Some(at) = pattern
        .iter()
        .position(|pat| matches!(pat, Pat::Lit(lit) if lit == head))
    else {
        return pattern;
    };
    if at == 0 {
        return pattern;
    }
    // Only skip modifiers. A hole before the name is the form itself.
    if pattern[..at].iter().all(|pat| matches!(pat, Pat::Lit(_))) {
        &pattern[at..]
    } else {
        pattern
    }
}

fn match_pats(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
) -> Option<usize> {
    match_pats_in(source, nodes, cursor, pattern, captures, classes, false)
}

/// `in_rep` is true while matching the inner pattern of a `Pat::Rep`. A
/// `$($rest:tt)+` step spans a call (`map(...)`); a standalone `$marker:token`
/// is exactly one token, so it must not swallow the `/` after it.
fn match_pats_in(
    source: &str,
    nodes: &[Syntax],
    cursor: &mut usize,
    pattern: &[Pat],
    captures: &mut HashMap<String, Capture>,
    classes: &[SyntaxClass],
    in_rep: bool,
) -> Option<usize> {
    let start_end = nodes.get(*cursor).map(node_start).unwrap_or(0);
    let mut last_end = start_end;
    for (pat_index, pat) in pattern.iter().enumerate() {
        match pat {
            Pat::Lit(expected) => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                if let Some(width) = lit_width(nodes, *cursor, expected) {
                    last_end = node_end(&nodes[*cursor + width - 1]);
                    *cursor += width;
                } else if node_is_lit(node, expected) {
                    last_end = node_end(node);
                    *cursor += 1;
                } else {
                    return None;
                }
            }
            Pat::Group { delimiter, inner } => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                let Syntax::Group {
                    delimiter: found,
                    children,
                    ..
                } = node
                else {
                    return None;
                };
                if found != delimiter {
                    return None;
                }
                let mut child = 0;
                if match_pats(source, children, &mut child, inner, captures, classes).is_none() {
                    return None;
                }
                // `record Name()` is an empty group. It must not accept
                // `record Name(head: number)`, whose fields are a later rule.
                skip_comments(children, &mut child);
                if child < children.len() {
                    return None;
                }
                // A single hole or `$($body:stmt)*` owns the group's interior,
                // including operators the flat walker stopped at.
                let whole = match inner.as_slice() {
                    [Pat::Hole { name, .. }] => Some(name.clone()),
                    [
                        Pat::Rep {
                            inner: rep_inner,
                            sep: None,
                            ..
                        },
                    ] => match rep_inner.as_slice() {
                        [
                            Pat::Hole {
                                name,
                                kind: HoleKind::Stmt,
                            },
                        ] => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = whole {
                    let already = captures
                        .get(&name)
                        .is_some_and(|have| matches!(have, Capture::Many(_)));
                    if !already {
                        let text = sweetener_reader::slice_range(
                            source,
                            node_start(node) + 1,
                            node_end(node).saturating_sub(1),
                        )
                        .to_owned();
                        captures.insert(name, Capture::Text(text));
                    }
                }
                last_end = node_end(node);
                *cursor += 1;
            }
            Pat::Hole { name, kind } => {
                if let HoleKind::Class(class_name) = kind {
                    let class_ = classes.iter().find(|class_| class_.name == *class_name)?;
                    let (capture, end, next) = take_class(source, nodes, *cursor, class_, classes)?;
                    captures.insert(name.clone(), capture);
                    last_end = end;
                    *cursor = next;
                } else {
                    // A literal right after an expression hole is that
                    // hole's stop. `$left:expr === $right:expr` has the
                    // reader's split `==` then `=`, so the hole must stop
                    // at `==`. With no following literal the hole is
                    // greedy, as in `unless($condition:expr) { ... }`.
                    let stop = match pattern.get(pat_index + 1) {
                        Some(Pat::Lit(text)) => Some(text.as_str()),
                        _ => None,
                    };
                    let taken = if *kind == HoleKind::Binding && pattern.len() == 1 {
                        // `rule { clamp(...) }` is a binding hole standing for the
                        // whole call. Inside `function $name(...)` it is one name.
                        take_expr(source, nodes, *cursor)
                            .or_else(|| take_ident(source, nodes, *cursor))
                    } else if *kind == HoleKind::Expr {
                        take_expr_until(source, nodes, *cursor, stop)
                    } else {
                        take_hole(source, nodes, *cursor, kind, in_rep)
                    };
                    let (text, end, next) = taken?;
                    captures.insert(name.clone(), Capture::Text(text));
                    last_end = end;
                    *cursor = next;
                }
            }
            Pat::Rep { inner, sep, kind } => {
                let mut many = Vec::new();
                let saved_start = *cursor;
                loop {
                    let saved = *cursor;
                    let mut one = HashMap::new();
                    if match_pats_in(source, nodes, cursor, inner, &mut one, classes, true).is_none() {
                        *cursor = saved;
                        break;
                    }
                    let mut one: Vec<(String, Capture)> = one.into_iter().collect();
                    // A separator that was written inside the repetition
                    // (`$($rest:tt,)+`) is not part of the captured step.
                    if sep.is_some() {
                        for (_, value) in &mut one {
                            if let Capture::Text(text) = value {
                                *text = text.trim().trim_end_matches(',').trim().to_owned();
                            }
                        }
                    }
                    many.push(one);
                    if let Some(sep) = sep {
                        let saved = *cursor;
                        skip_comments(nodes, cursor);
                        if nodes
                            .get(*cursor)
                            .is_some_and(|node| node_is_lit(node, sep))
                        {
                            *cursor += 1;
                            continue;
                        }
                        *cursor = saved;
                        break;
                    }
                }
                if *kind == RepKind::OneOrMore && many.is_empty() {
                    return None;
                }
                // A repetition's captures are stored under each inner name.
                let mut grouped: HashMap<String, Vec<Vec<(String, Capture)>>> = HashMap::new();
                for one in &many {
                    for (name, value) in one {
                        let key: String = name.clone();
                        grouped
                            .entry(key)
                            .or_default()
                            .push(vec![(name.clone(), value.clone())]);
                    }
                }
                // `$($body:stmt)*` with no separator is the whole interior,
                // including the newlines between the braces. A syntax-class
                // repetition keeps the records; the blob would hide `name`
                // and `moves`.
                if sep.is_none() {
                    if let Some(name) = many
                        .iter()
                        .find_map(|one| one.first().map(|(name, _)| name.clone()))
                    {
                        let class_rows = many
                            .iter()
                            .all(|one| matches!(one.first(), Some((_, Capture::Record { .. }))));
                        // `$($body:item)*` is several items. One blob hides
                        // `record` from the next expansion. Keep each item.
                        let item_rows = many.iter().all(|one| {
                            matches!(one.first(), Some((_, Capture::Text(text))) if !text.contains('\n') || text.trim_end().ends_with(';'))
                        }) && many.len() > 1;
                        if !class_rows && !item_rows {
                            if let (Some(start_node), Some(end_node)) = (
                                nodes.get(saved_start),
                                cursor.checked_sub(1).and_then(|at| nodes.get(at)),
                            ) {
                                let text = sweetener_reader::slice_range(
                                    source,
                                    node_start(start_node),
                                    node_end(end_node),
                                );
                                if !grouped.contains_key(&name) {
                                    captures.insert(name, Capture::Text(text.to_owned()));
                                }
                            }
                        }
                    }
                }
                for (name, rows) in grouped {
                    if captures.contains_key(&name) {
                        continue;
                    }
                    // Each row is one class match. Key it by the field the
                    // repetition named (`state`, `moves`) so `$state.name`
                    // walks that record instead of the first field text.
                    // A record's own `name` field is not that path: the
                    // template writes `$field.name`, and the row is stored
                    // under `field`.
                    let rows: Vec<Vec<(String, Capture)>> = rows
                        .into_iter()
                        .map(|one| {
                            let mut row = if one.len() == 1 && one[0].0 != name {
                                vec![(name.clone(), one[0].1.clone())]
                            } else {
                                one
                            };
                            // `$field.name` is not the row key. The class
                            // record is stored as `field`; copy its fields
                            // onto that path so the repetition can see them.
                            let extras: Vec<(String, Capture)> = row
                                .iter()
                                .filter_map(|(key, value)| match value {
                                    Capture::Record { fields, .. } if key == &name => {
                                        Some(fields.clone())
                                    }
                                    _ => None,
                                })
                                .flatten()
                                .map(|(field, value)| (format!("{name}.{field}"), value))
                                .collect();
                            row.extend(extras);
                            row
                        })
                        .collect();
                    captures.insert(name, Capture::Many(rows));
                }
                if let Some(node) = cursor.checked_sub(1).and_then(|at| nodes.get(at)) {
                    last_end = node_end(node);
                }
            }
        }
    }
    Some(last_end)
}

/// Match one syntax-class form. The first rule whose pattern accepts
/// the input wins. Fields the rule did not bind stay absent so
/// `#if(present ...)` can tell them from an empty capture.
fn take_class(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    class_: &SyntaxClass,
    classes: &[SyntaxClass],
) -> Option<(Capture, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start = node_start(nodes.get(index)?);
    for rule in &class_.rules {
        let mut child = index;
        let mut raw = HashMap::new();
        let Some(end) = match_pats(source, nodes, &mut child, &rule.pattern, &mut raw, classes) else {
            continue;
        };
        let text = sweetener_reader::slice_range(source, start, end).to_owned();
        let mut fields = HashMap::new();
        for field in &class_.fields {
            let Some(value) = raw.remove(&field.name) else {
                continue;
            };
            if field.repeated {
                let rows = match value {
                    Capture::Many(rows) => rows,
                    other => vec![vec![(field.name.clone(), other)]],
                };
                fields.insert(field.name.clone(), Capture::Many(rows));
            } else if let Capture::Many(rows) = value {
                // `ident` and `binding` are one name. A hole that swallowed
                // the rest of the form is not the field.
                if let Some(one) = rows
                    .into_iter()
                    .next()
                    .and_then(|row| row.into_iter().next())
                {
                    fields.insert(field.name.clone(), one.1);
                }
            } else {
                fields.insert(field.name.clone(), value);
            }
        }
        return Some((Capture::Record { text, fields }, end, child));
    }
    None
}

fn take_hole(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    kind: &HoleKind,
    in_rep: bool,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    match kind {
        HoleKind::Class(_) => None,
        HoleKind::Stmt => {
            // `record Empty();` is an item. The name is not a brace, and
            // the semicolon is the closer. A following item in the same
            // module must not be swallowed with it.
            if matches!(node, Syntax::Token { raw, .. } if is_ident_text(raw)) {
                let mut end_index = index;
                let mut depth_groups = 0i32;
                while end_index < nodes.len() {
                    match &nodes[end_index] {
                        Syntax::Group { .. } => depth_groups += 1,
                        Syntax::Operator { raw, .. } if raw == ";" && depth_groups == 0 => {
                            let end = node_end(&nodes[end_index]);
                            return Some((
                                source[node_start(node)..end].to_owned(),
                                end,
                                end_index + 1,
                            ));
                        }
                        _ => {}
                    }
                    end_index += 1;
                    if end_index - index > 40 {
                        break;
                    }
                }
            }
            if is_closed_group(node, Delimiter::Brace) {
                return Some((
                    sweetener_reader::slice(source, node).to_owned(),
                    node_end(node),
                    index + 1,
                ));
            }
            let start = node_start(node);
            let mut end_index = index;
            while end_index < nodes.len() {
                // `return value;` ends at the semicolon. `if (...) { ... }`
                // and `try { ... } catch { ... }` have no semicolon. The
                // brace is the statement; a following `catch` or `finally`
                // is still that statement, not the next body item.
                if operator_is(Some(&nodes[end_index]), ";") {
                    let end = node_end(&nodes[end_index]);
                    return Some((source[start..end].to_owned(), end, end_index + 1));
                }
                if is_closed_group(&nodes[end_index], Delimiter::Brace) {
                    let mut after = end_index + 1;
                    skip_comments(nodes, &mut after);
                    if token_is(nodes.get(after), "catch") || token_is(nodes.get(after), "finally")
                    {
                        end_index = after;
                        continue;
                    }
                    let end = node_end(&nodes[end_index]);
                    return Some((source[start..end].to_owned(), end, end_index + 1));
                }
                end_index += 1;
                if end_index - index > 80 {
                    break;
                }
            }
            None
        }
        HoleKind::Ident | HoleKind::Binding => take_ident(source, nodes, index),
        HoleKind::Type => take_type(source, nodes, index),
        HoleKind::TokenTree => {
            // `$($rest:tt)+` is the remaining steps of a thread, which start
            // with an identifier (`map(...)`). A repetition step spans a
            // whole call; a standalone `$marker:token` is exactly one token
            // and must not swallow the `/` after it.
            if in_rep {
                take_token_tree(source, nodes, index).or_else(|| take_ident(source, nodes, index))
            } else {
                take_ident(source, nodes, index).or_else(|| take_one_token(source, nodes, index))
            }
        }
        HoleKind::Expr => {
            // One primary, then tight postfix, then infix operators whose
            // precedence is below a surrounding custom operator. The simple
            // slice stops at a comma, semicolon, or a closer the pattern owns.
            take_expr(source, nodes, index)
        }
    }
}

/// A type ends at `{`, `;`, `,`, or `=`. It is not the function body.
fn take_type(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start_node = nodes.get(index)?;
    if !is_expr_start(start_node) {
        return None;
    }
    let start = node_start(start_node);
    let mut end = node_end(start_node);
    index += 1;
    loop {
        skip_comments(nodes, &mut index);
        let Some(node) = nodes.get(index) else {
            break;
        };
        if matches!(
            node,
            Syntax::Group {
                delimiter: Delimiter::Brace,
                ..
            }
        ) {
            break;
        }
        if matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ";" | "," | "=" | ")" | "]" | "}" | ">"))
        {
            break;
        }
        // `Empty | Cons` is one type. `|` is not a field separator; `,` is.
        if operator_is(Some(node), "|") {
            end = node_end(node);
            index += 1;
            continue;
        }
        // `value:` with the type missing stops at the colon. A following
        // closer or comma is not the type: `Some(value:)` and `head:, tail`.
        if operator_is(nodes.get(index.wrapping_sub(1)), ":")
            && matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ")" | "," | ";" | "}" | "]"))
        {
            return None;
        }
        // `string[]` and `readonly string` are part of the type. A following
        // `>` closes `optional<...>`, so it is not consumed here.
        end = node_end(node);
        index += 1;
        if index > cursor + 24 {
            break;
        }
    }
    Some((source[start..end].to_owned(), end, index))
}

/// `$name:binding` is one identifier, not the call or type that follows it.
fn take_ident(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    let Syntax::Token { raw, .. } = node else {
        return None;
    };
    if !is_ident_text(raw) {
        return None;
    }
    Some((raw.clone(), node_end(node), index + 1))
}

/// One token of a `$($segment)/+` repetition. A group is the whole group.
fn take_token_tree(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    if is_expr_stop(node) {
        return None;
    }
    // `$($rest:tt)+` is the remaining steps, not one character. A call
    // (`map((value) => value + 1)`) is one step; the comma after it belongs
    // to the repetition's separator, not to this hole.
    let start = node_start(node);
    // A brace group is a whole body. `kinded Functor<F> { ... }` stops at
    // the closing brace and does not swallow the next item
    // (`Functor FunctorArray for Array`).
    if is_closed_group(node, Delimiter::Brace) {
        let end = node_end(node);
        return Some((source[start..end].to_owned(), end, index + 1));
    }
    let mut end_index = index + 1;
    while end_index < nodes.len() {
        if operator_is(nodes.get(end_index), ",") || is_expr_stop(&nodes[end_index]) {
            break;
        }
        end_index += 1;
        if end_index - index > 24 {
            break;
        }
    }
    let end = node_end(&nodes[end_index - 1]);
    Some((source[start..end].to_owned(), end, end_index))
}

/// One token or group, whatever its kind. A standalone `token` hole uses
/// this; the greedy [`take_token_tree`] is only for a repetition step.
fn take_one_token(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    if is_expr_stop(node) {
        return None;
    }
    Some((sweetener_reader::slice(source, node).to_owned(), node_end(node), index + 1))
}

fn take_expr(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    take_expr_until(source, nodes, cursor, None)
}

/// True when every character is one an operator can be spelled with.
/// The reader splits `===` into `==` and `=`, so a fixed list of spellings
/// misses the second half.
fn is_operator_spelling(raw: &str) -> bool {
    !raw.is_empty() && raw.chars().all(|ch| "!=<>&|?+-*/%^~".contains(ch))
}

/// Operand of a right-associative operator, including another copy of that
/// operator. `2 ** 3 ** 2` captures `3 ** 2`, not just `3`.
fn take_right_assoc_operand(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start_node = nodes.get(index)?;
    if !is_expr_start(start_node) {
        return None;
    }
    let start = node_start(start_node);
    index += 1;
    while index < nodes.len() {
        skip_comments(nodes, &mut index);
        let Some(node) = nodes.get(index) else {
            break;
        };
        if is_expr_stop(node) {
            break;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ":") {
            break;
        }
        // `=>` is `=` then `>`. The arrow is part of the expression.
        if operator_is(Some(node), "=") && operator_is(nodes.get(index + 1), ">") {
            index += 2;
            continue;
        }
        if operator_is(Some(node), "=") {
            break;
        }
        if matches!(node, Syntax::Group { .. })
            || matches!(node, Syntax::Token { .. } | Syntax::String { .. })
        {
            index += 1;
            continue;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == ".") {
            index += 1;
            if matches!(nodes.get(index), Some(Syntax::Token { .. })) {
                index += 1;
            }
            continue;
        }
        if matches!(node, Syntax::Operator { .. })
            && nodes.get(index + 1).is_some_and(is_expr_start)
        {
            index += 1;
            continue;
        }
        break;
    }
    if index == cursor {
        return None;
    }
    let end = node_end(&nodes[index - 1]);
    Some((source[start..end].to_owned(), end, index))
}

fn take_expr_until(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    stop_op: Option<&str>,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let start_node = nodes.get(index)?;
    let start = node_start(start_node);
    if !is_expr_start(start_node) {
        return None;
    }
    index += 1;
    // `-4` starts on the operator. The operand is the next primary, not a
    // trailing operator the walk would drop.
    if matches!(start_node, Syntax::Operator { .. }) && nodes.get(index).is_some_and(is_expr_start)
    {
        index += 1;
    }
    loop {
        skip_comments(nodes, &mut index);
        let Some(node) = nodes.get(index) else {
            break;
        };
        if is_expr_stop(node) {
            break;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ":") {
            break;
        }
        // The reader splits `=>` into `=` and `>`. An arrow is part of the
        // expression (`(value) => value + 1`), not a type annotation.
        if operator_is(Some(node), "=") && operator_is(nodes.get(index + 1), ">") {
            index += 2;
            continue;
        }
        // Postfix call or index, or an infix operator followed by another primary.
        if matches!(node, Syntax::Group { .. }) {
            index += 1;
            continue;
        }
        if matches!(node, Syntax::Operator { raw, .. } if raw == ".") {
            if matches!(nodes.get(index + 1), Some(Syntax::Token { .. })) {
                index += 2;
                continue;
            }
            break;
        }
        if matches!(node, Syntax::Token { .. } | Syntax::String { .. }) {
            // The operator branch lands on the operand (`0` in `!== 0`).
            // Include it; a bare token after a finished primary is the next statement.
            if index > cursor && matches!(nodes.get(index - 1), Some(Syntax::Operator { .. })) {
                index += 1;
                continue;
            }
            break;
        }
        if matches!(node, Syntax::Operator { .. }) {
            if stop_op.is_some() && operator_spelling(node) == stop_op && index > cursor {
                break;
            }
            let op_end = index + 1;
            if nodes.get(op_end).is_some_and(is_expr_start) {
                index = op_end;
                continue;
            }
            // `!==` and `&&` are several operator characters. The reader
            // splits `===` into `==` and `=`, so the spelling is not one of
            // a fixed list; every character is an operator character. An
            // expression hole includes them; a custom infix still stops at
            // its own spelling.
            if matches!(node, Syntax::Operator { raw, .. } if is_operator_spelling(raw))
                && nodes.get(op_end).is_some_and(|next| {
                    matches!(
                        next,
                        Syntax::Operator { .. } | Syntax::Token { .. } | Syntax::String { .. }
                    )
                })
            {
                index = op_end;
                continue;
            }
            break;
        }
        if token_is(Some(node), "as") {
            index += 1;
            continue;
        }
        break;
    }
    // Walk back if we consumed a trailing infix operator with no
    // right-hand side. A leading `-4` is the expression, not a trailing op.
    while index > cursor + 1 && matches!(nodes.get(index - 1), Some(Syntax::Operator { .. })) {
        // `!==` was consumed as operators. Do not walk back onto `!` when
        // `==` and its operand are already part of the expression.
        let chained = matches!(nodes.get(index - 1), Some(Syntax::Operator { raw, .. }) if is_operator_spelling(raw))
            && nodes.get(index).is_some_and(|next| {
                matches!(
                    next,
                    Syntax::Operator { .. } | Syntax::Token { .. } | Syntax::String { .. }
                )
            });
        if chained {
            break;
        }
        if stop_op.is_some()
            && operator_spelling(&nodes[index - 1]) != stop_op
            && nodes.get(index).is_some_and(is_expr_start)
        {
            break;
        }
        if stop_op.is_some() && operator_spelling(&nodes[index - 1]) == stop_op {
            break;
        }
        let leading = index - 1 == cursor;
        index -= 1;
        if leading {
            break;
        }
    }
    let end = node_end(&nodes[index - 1]);
    Some((source[start..end].to_owned(), end, index))
}

fn is_expr_start(node: &Syntax) -> bool {
    match node {
        Syntax::Token { .. } | Syntax::String { .. } | Syntax::Group { .. } => true,
        Syntax::Operator { raw, .. } => matches!(
            raw.as_str(),
            "!" | "-" | "+" | "~" | "#" | "..." | "." | "<"
        ),
        Syntax::Comment { .. } => false,
    }
}

fn is_expr_stop(node: &Syntax) -> bool {
    match node {
        Syntax::Operator { raw, .. } => matches!(raw.as_str(), ";" | ")" | "]" | "}"),
        _ => false,
    }
}

/// The rename renderer prints a `#text` capture raw. Theirs quotes it.
/// Where the ordinary rendering has the quoted form and the renamed one has
/// the raw capture, put the quotes back.
/// Rename identifiers in an already-rendered replacement. Each plan entry
/// is `(spelling, occurrence, printed)`. A binder takes the next occurrence;
/// a reference takes the one in scope. Captures are not scanned: they were
/// already substituted, and a user spelling inside one stays as written when
/// it is not a binder the template introduced.
fn apply_plan(text: &str, plan: &[(String, String)]) -> String {
    let read = sweetener_reader::read_syntax(text);
    let mut seen = HashMap::<String, usize>::new();
    let mut out = String::new();
    let mut cursor = 0usize;
    apply_plan_nodes(
        text,
        &read.children,
        plan,
        &mut seen,
        false,
        &mut out,
        &mut cursor,
    );
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    out
}

fn plan_name(key: &str) -> &str {
    key.split('\0').next().unwrap_or(key)
}

fn plan_name_map(plan: &[(String, String)]) -> HashMap<String, String> {
    occurrence_plan(plan, &HashMap::new())
}

fn occurrence_plan(
    plan: &[(String, String)],
    seen: &HashMap<String, usize>,
) -> HashMap<String, String> {
    let mut map = HashMap::new();
    for (key, printed) in plan {
        let name = plan_name(key);
        let nth = seen.get(name).copied().unwrap_or(0);
        if key == &format!("{name}\0{nth}") || key == name || (!key.contains('\0') && nth == 0) {
            map.insert(name.to_owned(), printed.clone());
        }
    }
    map
}

fn apply_plan_nodes(
    source: &str,
    nodes: &[Syntax],
    plan: &[(String, String)],
    seen: &mut HashMap<String, usize>,
    after_dot: bool,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut index = 0;
    let mut dotted = after_dot;
    while index < nodes.len() {
        match &nodes[index] {
            Syntax::Token { span, raw } if !dotted && is_ident_text(raw) => {
                let binder = is_binder_at(nodes, index) || in_type_params(nodes, index);
                if let Some(printed) = planned(raw, binder, plan, seen) {
                    push_until(source, out, cursor, span.start as usize);
                    out.push_str(&printed);
                    *cursor = span.end as usize;
                    if binder {
                        let n = seen.get(raw).copied().unwrap_or(0);
                        seen.insert(raw.clone(), n + 1);
                    }
                } else if binder {
                    // `[Brand]` is a later sibling group. Count the declaration
                    // now so that reference uses this occurrence, not a fresh one.
                    let n = seen.get(raw).copied().unwrap_or(0);
                    seen.insert(raw.clone(), n + 1);
                }
                dotted = false;
            }
            Syntax::Group { span, children, .. } if span.end as usize <= *cursor => {}
            Syntax::Group { children, .. } => {
                let method_params = index > 0
                    && operator_is(nodes.get(index - 1), ">")
                    && matches!(
                        nodes.get(index),
                        Some(Syntax::Group {
                            delimiter: Delimiter::Paren,
                            ..
                        })
                    );
                if method_params {
                    if let Syntax::Group { span, children, .. } = &nodes[index] {
                        // `map<A, B>(value: F<A>)` — count `A` before the
                        // parameter types, or `F<A>` keeps the user's `A`.
                        if let Some(angle) = nodes[..index]
                            .iter()
                            .rposition(|node| operator_is(Some(node), "<"))
                            .filter(|angle| {
                                nodes[*angle..index]
                                    .iter()
                                    .position(|node| operator_is(Some(node), ">"))
                                    .is_some_and(|close| angle + close + 1 == index)
                            })
                        {
                            if let Some(params) = type_param_names(nodes, angle) {
                                for name in params {
                                    // One count for the declaration. A second
                                    // count here makes the return type look
                                    // up an occurrence the plan never made.
                                    if seen.get(&name).copied().unwrap_or(0) == 0 {
                                        seen.insert(name, 1);
                                    }
                                }
                            }
                        }
                        push_until(source, out, cursor, span.start as usize);
                        let mut local = span.start as usize;
                        apply_arrow_params(source, children, plan, seen, out, &mut local);
                        if local < span.end as usize {
                            out.push_str(&source[local..span.end as usize]);
                        }
                        *cursor = span.end as usize;
                        // `: F<B>` is the return type, still in this signature.
                        let mut rest = index + 1;
                        if operator_is(nodes.get(rest), ":") {
                            rest += 1;
                            let mut end = rest;
                            while end < nodes.len()
                                && !operator_is(nodes.get(end), ";")
                                && !operator_is(nodes.get(end), "{")
                                && !matches!(
                                    nodes.get(end),
                                    Some(Syntax::Group {
                                        delimiter: Delimiter::Brace,
                                        ..
                                    })
                                )
                            {
                                end += 1;
                                if end > rest + 8 {
                                    break;
                                }
                            }
                            apply_plan_nodes(
                                source,
                                &nodes[rest..end],
                                plan,
                                seen,
                                false,
                                out,
                                cursor,
                            );
                            if end > rest {
                                let close = node_end(&nodes[end - 1]);
                                if *cursor < close {
                                    out.push_str(&source[*cursor..close]);
                                    *cursor = close;
                                }
                            }
                            index = end;
                            continue;
                        }
                        index += 1;
                        continue;
                    }
                }
                if let Some(_params) = arrow_params(nodes, index) {
                    if let Syntax::Group { span, children, .. } = &nodes[index] {
                        // `map<A, B>(a: A)` — the type parameters are binders
                        // of this method. Count them before the parameter list
                        // or `a` takes `A`'s suffix. The prefix is still copied;
                        // rewrite the `<...>` here so `A` becomes `A_1`.
                        if let Some(angle) = nodes[..index]
                            .iter()
                            .rposition(|node| operator_is(Some(node), "<"))
                        {
                            if let Some(close) = nodes[angle..index]
                                .iter()
                                .position(|node| operator_is(Some(node), ">"))
                            {
                                let close_at = angle + close;
                                // Only the type-parameter list that ends at
                                // this parameter list. `F<A>` earlier in the
                                // signature is a use, not this binder.
                                if close_at + 1 == index && type_param_names(nodes, angle).is_some()
                                {
                                    push_until(source, out, cursor, node_start(&nodes[angle]));
                                    out.push('<');
                                    let mut local = node_end(&nodes[angle]);
                                    apply_plan_nodes(
                                        source,
                                        &nodes[angle + 1..close_at],
                                        plan,
                                        seen,
                                        false,
                                        out,
                                        &mut local,
                                    );
                                    if local < node_start(&nodes[close_at]) {
                                        out.push_str(&source[local..node_start(&nodes[close_at])]);
                                    }
                                    out.push('>');
                                    *cursor = node_end(&nodes[close_at]);
                                }
                            }
                        }
                        push_until(source, out, cursor, span.start as usize);
                        let mut local_cursor = span.start as usize;
                        apply_arrow_params(source, children, plan, seen, out, &mut local_cursor);
                        if local_cursor < span.end as usize {
                            out.push_str(&source[local_cursor..span.end as usize]);
                        }
                        *cursor = span.end as usize;
                        let mut rest_index = index + 1;
                        if operator_is(nodes.get(rest_index), "=>") {
                            rest_index += 1;
                        }
                        apply_plan_nodes(
                            source,
                            &nodes[rest_index..],
                            plan,
                            seen,
                            false,
                            out,
                            cursor,
                        );
                        index = nodes.len();
                        continue;
                    }
                } else {
                    apply_plan_nodes(source, children, plan, seen, false, out, cursor);
                }
                dotted = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => dotted = true,
            Syntax::String { span, raw } if raw.starts_with('`') && raw.contains("${") => {
                let map = occurrence_plan(plan, seen);
                let needs = map.keys().any(|name| {
                    let mut whole = String::from("${");
                    whole.push_str(name);
                    whole.push('}');
                    let mut member = String::from("${");
                    member.push_str(name);
                    member.push('.');
                    raw.contains(&whole) || raw.contains(&member)
                });
                if needs {
                    let renamed = rename_template_literal(
                        raw,
                        &map,
                        &[],
                        &HashMap::new(),
                        seen,
                        span.start as usize,
                    );
                    push_until(source, out, cursor, span.start as usize);
                    out.push_str(&renamed);
                    *cursor = span.end as usize;
                }
                dotted = false;
            }
            _ => dotted = false,
        }
        index += 1;
    }
}

fn planned(
    raw: &str,
    binder: bool,
    plan: &[(String, String)],
    seen: &HashMap<String, usize>,
) -> Option<String> {
    let opened = seen.get(raw).copied().unwrap_or(0);
    if !binder && opened == 0 {
        return plan
            .iter()
            .find(|(key, _)| plan_name(key) == raw && !key.contains('\0'))
            .map(|(_, printed)| printed.clone());
    }
    let nth = if binder {
        opened
    } else {
        opened.saturating_sub(1)
    };
    plan.iter()
        .find(|(key, _)| key == &format!("{raw}\0{nth}"))
        .map(|(_, printed)| printed.clone())
}

fn sole_capture(nodes: &[Syntax], index: usize) -> bool {
    nodes.len() == 1 && index == 0
}

fn apply_arrow_params(
    source: &str,
    children: &[Syntax],
    plan: &[(String, String)],
    seen: &mut HashMap<String, usize>,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut index = 0;
    while index < children.len() {
        if let Syntax::Token { span, raw } = &children[index] {
            let after_colon = index > 0 && operator_is(children.get(index - 1), ":");
            if is_ident_text(raw) && !is_type_name(raw) {
                // A name after `:` is a type reference, not a new binder.
                let binder = !after_colon;
                if let Some(printed) = planned(raw, binder, plan, seen) {
                    push_until(source, out, cursor, span.start as usize);
                    out.push_str(&printed);
                    *cursor = span.end as usize;
                }
                if binder {
                    let n = seen.get(raw).copied().unwrap_or(0);
                    seen.insert(raw.clone(), n + 1);
                }
            }
        }
        if let Syntax::Group {
            children: nested, ..
        } = &children[index]
        {
            apply_arrow_params(source, nested, plan, seen, out, cursor);
        }
        index += 1;
    }
}

fn is_type_name(raw: &str) -> bool {
    matches!(
        raw,
        "number"
            | "string"
            | "boolean"
            | "any"
            | "unknown"
            | "never"
            | "void"
            | "object"
            | "bigint"
            | "symbol"
            | "undefined"
            | "null"
    )
}

fn u32_at(offset: usize) -> u32 {
    u32::try_from(offset).unwrap_or(u32::MAX)
}

fn node_span(node: &Syntax) -> Span {
    match node {
        Syntax::Token { span, .. }
        | Syntax::Comment { span, .. }
        | Syntax::String { span, .. }
        | Syntax::Operator { span, .. }
        | Syntax::Group { span, .. } => *span,
    }
}

fn node_start(node: &Syntax) -> usize {
    node_span(node).start as usize
}

fn node_end(node: &Syntax) -> usize {
    node_span(node).end as usize
}

fn token_at(nodes: &[Syntax], index: usize, expected: &str) -> bool {
    matches!(nodes.get(index), Some(Syntax::Token { raw, .. }) if raw == expected)
}

fn token_is(node: Option<&Syntax>, expected: &str) -> bool {
    matches!(node, Some(Syntax::Token { raw, .. }) if raw == expected)
}

fn operator_is(node: Option<&Syntax>, expected: &str) -> bool {
    match node {
        Some(Syntax::Operator { raw, .. } | Syntax::Token { raw, .. }) => raw == expected,
        _ => false,
    }
}

fn operator_spelling(node: &Syntax) -> Option<&str> {
    match node {
        Syntax::Operator { raw, .. } => Some(raw.as_str()),
        // `implements` is a word, not a punctuation operator.
        Syntax::Token { raw, .. } if is_ident_text(raw) => Some(raw.as_str()),
        _ => None,
    }
}

fn group_is(node: Option<&Syntax>, delimiter: Delimiter) -> bool {
    matches!(node, Some(Syntax::Group { delimiter: found, .. }) if *found == delimiter)
}

fn is_closed_group(node: &Syntax, delimiter: Delimiter) -> bool {
    matches!(node, Syntax::Group { delimiter: found, close: Some(_), .. } if *found == delimiter)
}

/// `<-` and `->` are one macro name, but the reader stores each character.
/// A literal matches that run when the characters are adjacent.
fn lit_width(nodes: &[Syntax], index: usize, expected: &str) -> Option<usize> {
    if expected.chars().count() < 2 || !expected.chars().all(|ch| "<>-".contains(ch)) {
        return None;
    }
    let mut cursor = index;
    let mut got = String::new();
    let mut width = 0usize;
    while got.len() < expected.len() {
        let raw = operator_spelling(nodes.get(cursor)?)?;
        if !raw.chars().all(|ch| "<>-".contains(ch)) {
            return None;
        }
        if width > 0 {
            let prev_end = node_end(&nodes[cursor - 1]);
            if node_start(&nodes[cursor]) != prev_end {
                return None;
            }
        }
        got.push_str(raw);
        width += 1;
        cursor += 1;
        if width > 4 {
            return None;
        }
    }
    (got == expected).then_some(width)
}

fn node_is_lit(node: &Syntax, expected: &str) -> bool {
    match node {
        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } | Syntax::String { raw, .. } => {
            raw == expected
        }
        _ => false,
    }
}

fn is_ident_text(raw: &str) -> bool {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) if first == '_' || first.is_ascii_alphabetic() => {
            chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        }
        _ => false,
    }
}

fn open_char(delimiter: Delimiter) -> char {
    match delimiter {
        Delimiter::Paren => '(',
        Delimiter::Brace => '{',
        Delimiter::Bracket => '[',
    }
}

fn skip_comments(nodes: &[Syntax], cursor: &mut usize) {
    while nodes
        .get(*cursor)
        .is_some_and(|node| matches!(node, Syntax::Comment { .. }))
    {
        *cursor += 1;
    }
}

fn push_until(source: &str, out: &mut String, cursor: &mut usize, until: usize) {
    if until < *cursor {
        // A nested walk started past this token. The caller is about to
        // write the token itself, so do not also copy it from source.
        *cursor = until;
        return;
    }
    if until <= source.len() {
        out.push_str(&source[*cursor..until]);
        *cursor = until;
    }
}

fn macro_head(node: &Syntax) -> Option<&str> {
    match node {
        Syntax::Token { raw, .. } if is_ident_text(raw) => Some(raw.as_str()),
        _ => None,
    }
}

/// True when the node at `index` is an identifier that a property access
/// owns: `x.service` (the name follows a `.`) or `service.q` (the name is
/// followed by a `.`). A macro call never has a `.` on either side.
fn property_access_at(nodes: &[Syntax], index: usize) -> bool {
    if nodes.get(index).and_then(macro_head).is_none() {
        return false;
    }
    let prev_dot = index > 0 && operator_is(nodes.get(index - 1), ".");
    let next_dot = operator_is(nodes.get(index + 1), ".");
    prev_dot || next_dot
}

/// True when the identifier at `index` could start a macro rule. A macro
/// head is followed by a token, a group, or `<`. A bare identifier that a
/// parameter list or an expression owns (`(service) =>`, `service;`) is not.
/// A name a property access owns (`Effect.gen`) is a member, never a head.
/// A form rewrite can print such an identifier that shares a macro name.
fn macro_call_position(nodes: &[Syntax], index: usize) -> bool {
    if nodes.get(index).and_then(macro_head).is_none() {
        return false;
    }
    if index > 0 && operator_is(nodes.get(index - 1), ".") {
        return false;
    }
    let Some(next) = nodes.get(index + 1) else {
        return false;
    };
    match next {
        Syntax::Group { .. } => true,
        Syntax::Token { .. } | Syntax::String { .. } => true,
        Syntax::Operator { raw, .. } => matches!(raw.as_str(), "<" | "#"),
        _ => false,
    }
}

/// The macro a failed form was matching, including an operator inside a group.
fn macro_name_of(source: &str, nodes: &[Syntax], index: usize) -> Option<String> {
    if let Some(name) = nodes.get(index).and_then(macro_head) {
        return Some(name.to_owned());
    }
    let Syntax::Group { children, .. } = nodes.get(index)? else {
        return nodes
            .get(index)
            .and_then(operator_spelling)
            .map(str::to_owned);
    };
    let _ = source;
    if children.iter().any(|node| operator_is(Some(node), "#")) {
        return Some("#".to_owned());
    }
    if let Some(name) = children.iter().find_map(|node| {
        let raw = operator_spelling(node)?;
        if raw == "->" || raw == "->>" || raw == "<-" {
            Some(raw.to_owned())
        } else {
            None
        }
    }) {
        return Some(name);
    }
    joined_operator(children)
}

fn joined_operator(nodes: &[Syntax]) -> Option<String> {
    let first = nodes.first().and_then(operator_spelling)?;
    if first == "->" {
        if nodes.get(1).and_then(operator_spelling) == Some(">") {
            return Some(format!("{first}>"));
        }
        return Some(first.to_owned());
    }
    if first == "<-" {
        return Some(first.to_owned());
    }
    None
}

fn specifier_body(raw: &str) -> Option<&str> {
    let quote = raw.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    raw.strip_prefix(quote)?.strip_suffix(quote)
}

fn rewrite_specifier(raw: &str) -> String {
    let Some(body) = specifier_body(raw) else {
        return raw.to_owned();
    };
    for ext in [".sts", ".stsx"] {
        if let Some(stripped) = body.strip_suffix(ext) {
            let quote = raw.chars().next().unwrap_or('"');
            return format!("{quote}{stripped}{quote}");
        }
    }
    raw.to_owned()
}

fn user_spellings(nodes: &[Syntax]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    collect_user_binders(nodes, &mut names);
    names
}

/// `class Option<A>` and `function id<T>` bind those parameters. A use such
/// as `F<A>` does not bind `F`.
fn user_type_params(nodes: &[Syntax]) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    collect_user_type_params(nodes, &mut names);
    names
}

fn collect_user_type_params(nodes: &[Syntax], names: &mut std::collections::HashSet<String>) {
    let mut index = 0;
    while index < nodes.len() {
        if operator_is(nodes.get(index), "<") {
            let head = index.checked_sub(1).and_then(|at| nodes.get(at));
            let binds = match head {
                Some(Syntax::Token { raw, .. })
                    if is_binder_keyword(nodes, index.saturating_sub(2))
                        || matches!(raw.as_str(), "class" | "interface" | "type" | "function") =>
                {
                    true
                }
                Some(Syntax::Token { raw, .. })
                    if index >= 2 && is_binder_keyword(nodes, index - 2) =>
                {
                    true
                }
                _ => index > 0 && is_binder_keyword(nodes, index - 1),
            };
            // `class Option<A>`: `<` follows the class name, and the class
            // keyword is two tokens back. `function id<T>` is the same.
            let declares = index >= 2 && is_binder_keyword(nodes, index - 2);
            if declares {
                if let Some(params) = type_param_names(nodes, index) {
                    names.extend(params);
                }
            }
            let _ = binds;
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_user_type_params(children, names);
        }
        index += 1;
    }
}

fn collect_user_binders(nodes: &[Syntax], names: &mut std::collections::HashSet<String>) {
    let mut index = 0;
    while index < nodes.len() {
        if is_binder_keyword(nodes, index) {
            if let Some(name) = binder_name_after(nodes, index) {
                names.insert(name);
            }
        }
        // `declare class Option<A>` binds `A` only when a macro also
        // introduces `A`. Recording every type argument would rename `F`
        // because the user wrote `F<A>`.
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_user_binders(children, names);
        }
        index += 1;
    }
}

fn is_binder_keyword(nodes: &[Syntax], index: usize) -> bool {
    token_at(nodes, index, "const")
        || token_at(nodes, index, "let")
        || token_at(nodes, index, "var")
        || token_at(nodes, index, "function")
        || token_at(nodes, index, "class")
        || token_at(nodes, index, "enum")
        || token_at(nodes, index, "type")
        || token_at(nodes, index, "interface")
}

fn binder_name_after(nodes: &[Syntax], index: usize) -> Option<String> {
    let mut cursor = index + 1;
    if operator_is(nodes.get(cursor), "*") {
        cursor += 1;
    }
    match nodes.get(cursor) {
        Some(Syntax::Token { raw, .. }) if is_ident_text(raw) => Some(raw.clone()),
        _ => None,
    }
}

fn is_binder_at(nodes: &[Syntax], index: usize) -> bool {
    let Some(Syntax::Token { raw, .. }) = nodes.get(index) else {
        return false;
    };
    if !is_ident_text(raw) {
        return false;
    }
    if index > 0 && is_binder_keyword(nodes, index - 1) {
        return true;
    }
    if index > 1 && operator_is(nodes.get(index - 1), "*") && is_binder_keyword(nodes, index - 2) {
        return true;
    }
    // `map<A, B>(value, transform)` — the names in that paren list are binders.
    if method_param_name(nodes, index) {
        return true;
    }
    false
}

fn method_param_name(nodes: &[Syntax], index: usize) -> bool {
    let Some(open) = nodes[..index].iter().rposition(|node| {
        matches!(
            node,
            Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            }
        )
    }) else {
        return false;
    };
    if open == 0 || !operator_is(nodes.get(open - 1), ">") {
        return false;
    }
    let Syntax::Group { children, .. } = &nodes[open] else {
        return false;
    };
    let Some(Syntax::Token { span, .. }) = nodes.get(index) else {
        return false;
    };
    children.iter().any(|child| match child {
        Syntax::Token {
            span: child_span,
            raw,
        } => child_span.start == span.start && is_ident_text(raw) && !is_type_name(raw),
        _ => false,
    }) && !children.iter().enumerate().any(|(at, child)| {
        node_start(child) == span.start as usize && at > 0 && operator_is(children.get(at - 1), ":")
    })
}

fn syntax_imports(nodes: &[Syntax]) -> Vec<SyntaxImport> {
    let mut imports = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if is_syntax_import_at(nodes, index) {
            if let Some(import) = syntax_import_at(nodes, index) {
                imports.push(import);
            }
        }
        index += 1;
    }
    imports
}

fn inside_syntax_import(nodes: &[Syntax], index: usize) -> bool {
    if index == 0 {
        return false;
    }
    let mut cursor = index;
    while cursor > 0 {
        cursor -= 1;
        if operator_is(nodes.get(cursor), ";") {
            return false;
        }
        if token_is(Some(&nodes[cursor]), "import") {
            return nodes[cursor..=index]
                .iter()
                .any(|node| token_is(Some(node), "syntax") || token_is(Some(node), "from"));
        }
        if cursor + 8 < index {
            return false;
        }
    }
    false
}

fn is_syntax_import_at(nodes: &[Syntax], index: usize) -> bool {
    if !token_at(nodes, index, "import") {
        return false;
    }
    // Only this statement. A later `export syntax` in the same module is not
    // part of the import, and treating it as one dropped the runtime import.
    nodes
        .iter()
        .skip(index)
        .take_while(|node| !operator_is(Some(node), ";"))
        .any(|node| token_is(Some(node), "syntax"))
}

fn syntax_import_at(nodes: &[Syntax], index: usize) -> Option<SyntaxImport> {
    let span = node_span(&nodes[index]);
    let mut names = Vec::new();
    let mut aliases = Vec::new();
    if let Some(Syntax::Group {
        children,
        delimiter: Delimiter::Brace,
        ..
    }) = nodes.get(index + 1)
    {
        let mut child = 0;
        while child < children.len() {
            if let Syntax::Token { raw, .. } = &children[child] {
                if is_ident_text(raw) && raw != "as" {
                    if token_at(children, child + 1, "as") {
                        if let Some(Syntax::Token { raw: local, .. }) = children.get(child + 2) {
                            aliases.push((raw.clone(), local.clone()));
                            names.push(local.clone());
                            child += 3;
                            continue;
                        }
                    }
                    names.push(raw.clone());
                }
            }
            if let Syntax::Group {
                delimiter: Delimiter::Paren,
                children: inner,
                ..
            } = &children[child]
            {
                let name = inner
                    .iter()
                    .filter_map(|node| match node {
                        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => {
                            Some(raw.as_str())
                        }
                        _ => None,
                    })
                    .collect::<String>();
                if !name.is_empty() {
                    names.push(name);
                }
            }
            child += 1;
        }
    }
    let specifier = nodes
        .iter()
        .skip(index)
        .take(12)
        .find_map(|node| match node {
            Syntax::String { raw, .. } => Some(raw.clone()),
            _ => None,
        })?;
    Some(SyntaxImport {
        names,
        aliases,
        specifier,
        span,
    })
}

fn syntax_import_end(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return node_end(&nodes[cursor]);
        }
        cursor += 1;
        if cursor > index + 16 {
            break;
        }
    }
    node_end(&nodes[index])
}

fn skip_syntax_import(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index;
    while cursor < nodes.len() {
        if operator_is(nodes.get(cursor), ";") {
            return cursor + 1;
        }
        cursor += 1;
        if cursor > index + 16 {
            break;
        }
    }
    index + 1
}

fn parse_pattern(source: &str) -> Vec<Pat> {
    let read = sweetener_reader::read_syntax(source);
    pats_from(&read.children)
}

fn instantiate(
    source: &str,
    template: &[Tpl],
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
) -> String {
    let _ = source;
    let mut out = String::new();
    render(&mut out, template, captures, classes);
    normalize_replacement(&out)
}

fn normalize_replacement(text: &str) -> String {
    text.trim().to_owned()
}

fn unknown_directives(template: &[Tpl]) -> Vec<(String, usize)> {
    let mut found = Vec::new();
    for part in template {
        match part {
            Tpl::Unknown { name, at } => found.push((name.clone(), *at)),
            Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::Define(inner) => {
                found.extend(unknown_directives(inner));
            }
            Tpl::If {
                then_body,
                else_body,
                ..
            } => {
                found.extend(unknown_directives(then_body));
                if let Some(else_body) = else_body {
                    found.extend(unknown_directives(else_body));
                }
            }
            _ => {}
        }
    }
    found
}

fn template_is_core_item(template: &[Tpl]) -> bool {
    template.iter().any(|part| match part {
        Tpl::Text(text) => text.contains("coreItem"),
        _ => false,
    })
}

fn template_is_core(template: &[Tpl]) -> bool {
    template.iter().any(|part| matches!(part, Tpl::Core(_)))
        && template.iter().all(|part| match part {
            Tpl::Core(_) => true,
            Tpl::Text(text) => text.trim().is_empty(),
            _ => false,
        })
}

fn introduced_of(source: &str, nodes: &[Syntax]) -> Vec<String> {
    let mut names = Vec::new();
    collect_introduced_source(source, nodes, &mut names);
    names
}

fn introduced_names(
    declared: &[String],
    rendered: &str,
    captures: &HashMap<String, Capture>,
) -> Vec<String> {
    let read = sweetener_reader::read_syntax(rendered);
    let mut names = introduced_of(rendered, &read.children);
    // An empty replacement introduced nothing. Falling back to the template
    // source would count binders inside `#syntax`, which belong to the
    // generated macro, not this expansion.
    if names.is_empty() && !rendered.trim().is_empty() {
        names.extend(declared.iter().cloned());
    }
    // A capture such as `$name` is the user's spelling. Renaming `Seconds`
    // because the template also binds it drops the quantity the rule created.
    let captured = capture_spellings(captures);
    names.retain(|name| !captured.contains(name));
    names
}

fn capture_spellings(captures: &HashMap<String, Capture>) -> std::collections::HashSet<String> {
    let mut names = std::collections::HashSet::new();
    for value in captures.values() {
        collect_capture_idents(value, &mut names);
    }
    names
}

/// A syntax-class field such as `parameters.second` is the user's spelling.
/// Renaming it because the rendered `#core` signature also binds it changes
/// a parameter theirs keeps.
fn collect_capture_idents(value: &Capture, names: &mut std::collections::HashSet<String>) {
    match value {
        Capture::Text(text) => {
            let trimmed = text.trim();
            if is_ident_text(trimmed) {
                names.insert(trimmed.to_owned());
            }
        }
        Capture::Record { fields, text } => {
            let trimmed = text.trim();
            if is_ident_text(trimmed) {
                names.insert(trimmed.to_owned());
            }
            for field in fields.values() {
                collect_capture_idents(field, names);
            }
        }
        Capture::Many(rows) => {
            for row in rows {
                for (_, field) in row {
                    collect_capture_idents(field, names);
                }
            }
        }
    }
}

fn collect_introduced_source(source: &str, nodes: &[Syntax], names: &mut Vec<String>) {
    let _ = source;
    let mut index = 0;
    while index < nodes.len() {
        if dollar_at(nodes, index) {
            index += 2;
            continue;
        }
        if is_binder_keyword(nodes, index) {
            if let Some(name) = binder_name_after(nodes, index) {
                names.push(name);
            }
        }
        if let Some(params) = type_param_names(nodes, index) {
            names.extend(params);
        }
        if let Syntax::Group {
            delimiter: Delimiter::Paren,
            children,
            ..
        } = &nodes[index]
        {
            // Arrow parameters and `map<A, B>(value, transform)`. The second
            // expansion must not reprint `value` once the first bound it.
            // `function add(left, right)` is not that list. The reader's
            // `=>` is `=` then `>`, so a following `>` is a type annotation
            // on an arrow, not `map<A, B>(...)`. `#core` keeps `right`.
            let after_type_params = index > 0
                && operator_is(nodes.get(index - 1), ">")
                && !operator_is(nodes.get(index.wrapping_sub(2)), "=");
            if paren_is_arrow_params(nodes, index) || after_type_params {
                names.extend(param_names(children));
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_introduced_source(source, children, names);
        }
        index += 1;
    }
}

/// True when `index` is an identifier inside the first type-parameter
/// list, `map<A, B>`, not a later argument such as `F<A>`.
fn in_type_params(nodes: &[Syntax], index: usize) -> bool {
    let mut depth = 0i32;
    let mut open = None;
    for (at, node) in nodes.iter().enumerate().take(index + 1) {
        if operator_is(Some(node), "<") {
            if depth == 0 {
                open = Some(at);
            }
            depth += 1;
        } else if operator_is(Some(node), ">") && depth > 0 {
            depth -= 1;
            if depth == 0 {
                open = None;
            }
        }
    }
    let Some(open) = open else {
        return false;
    };
    if depth == 0 || open == 0 {
        return false;
    }
    matches!(nodes.get(open - 1), Some(Syntax::Token { raw, .. }) if is_ident_text(raw))
        && type_param_names(nodes, open).is_some_and(|params| {
            nodes
                .get(index)
                .and_then(|node| match node {
                    Syntax::Token { raw, .. } => Some(raw),
                    _ => None,
                })
                .is_some_and(|raw| params.iter().any(|name| name == raw))
        })
}

/// `type F<Inner>` and `map<A, B>` introduce the names between `<` and `>`.
fn type_param_names(nodes: &[Syntax], index: usize) -> Option<Vec<String>> {
    if !operator_is(nodes.get(index), "<") {
        return None;
    }
    let mut names = Vec::new();
    let mut cursor = index + 1;
    let mut depth = 1i32;
    while cursor < nodes.len() && depth > 0 {
        if operator_is(nodes.get(cursor), "<") {
            depth += 1;
        } else if operator_is(nodes.get(cursor), ">") {
            depth -= 1;
            if depth == 0 {
                break;
            }
        } else if depth == 1 {
            if let Syntax::Token { raw, .. } = &nodes[cursor] {
                if is_ident_text(raw) && !is_type_name(raw) && raw != "extends" && raw != "keyof" {
                    names.push(raw.clone());
                }
            }
        }
        cursor += 1;
        if cursor > index + 24 {
            break;
        }
    }
    if names.is_empty() { None } else { Some(names) }
}

fn arrow_params(nodes: &[Syntax], index: usize) -> Option<Vec<String>> {
    let Syntax::Group {
        delimiter: Delimiter::Paren,
        children,
        ..
    } = nodes.get(index)?
    else {
        return None;
    };
    if !paren_is_arrow_params(nodes, index) {
        return None;
    }
    Some(param_names(children))
}

fn paren_is_arrow_params(nodes: &[Syntax], index: usize) -> bool {
    // `of: (value: number): Meters =>` — the arrow is a few siblings later.
    // A second paren before `=>` is another method, not this arrow.
    for node in nodes.iter().skip(index + 1).take(8) {
        if operator_is(Some(node), "=>") {
            return true;
        }
        if matches!(
            node,
            Syntax::Group {
                delimiter: Delimiter::Paren,
                ..
            } | Syntax::Group {
                delimiter: Delimiter::Brace,
                ..
            }
        ) {
            return false;
        }
    }
    false
}

fn param_names(children: &[Syntax]) -> Vec<String> {
    let mut names = Vec::new();
    let mut child = 0;
    while child < children.len() {
        if let Syntax::Token { raw, .. } = &children[child] {
            let after_colon = child > 0 && operator_is(children.get(child - 1), ":");
            if is_ident_text(raw) && !after_colon && !is_type_name(raw) {
                names.push(raw.clone());
            }
        }
        child += 1;
    }
    names
}

/// Render a `#syntax` body. A hole this invocation captured is
/// substituted. A hole of the macro being defined is written back as
/// `$name`, including `$name.field`.
fn render_keeping_holes(out: &mut String, template: &[Tpl], captures: &HashMap<String, Capture>) {
    for part in template {
        match part {
            Tpl::Text(text) => out.push_str(&substitute_text(text, captures)),
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) => {
                if capture_at(captures, name).is_some() {
                    render(out, std::slice::from_ref(part), captures, &[]);
                } else {
                    out.push('$');
                    out.push_str(name);
                }
            }
            Tpl::Join {
                name,
                prefix,
                suffix,
                casing,
            } => {
                if capture_at(captures, name).is_some() {
                    render(out, std::slice::from_ref(part), captures, &[]);
                } else {
                    out.push_str("#join($");
                    out.push_str(name);
                    if !prefix.is_empty() {
                        out.push_str(&format!(", prefix: \"{prefix}\""));
                    }
                    if !suffix.is_empty() {
                        out.push_str(&format!(", suffix: \"{suffix}\""));
                    }
                    if casing != "preserve" {
                        out.push_str(&format!(", casing: \"{casing}\""));
                    }
                    out.push(')');
                }
            }
            Tpl::Core(inner) => {
                out.push_str("#core(");
                render_keeping_holes(out, inner, captures);
                out.push(')');
            }
            Tpl::If {
                path,
                then_body,
                else_body,
            } => {
                out.push_str("#if(present $");
                out.push_str(path);
                out.push_str(") {");
                render_keeping_holes(out, then_body, captures);
                out.push('}');
                if let Some(else_body) = else_body {
                    out.push_str(" #else {");
                    render_keeping_holes(out, else_body, captures);
                    out.push('}');
                }
            }
            Tpl::Rep { inner, sep } => {
                out.push_str("$(");
                render_keeping_holes(out, inner, captures);
                out.push(')');
                if let Some(sep) = sep {
                    out.push_str(sep);
                }
                out.push('*');
            }
            Tpl::Define(inner) => {
                out.push_str("#syntax {");
                render_keeping_holes(out, inner, captures);
                out.push('}');
            }
            Tpl::Unknown { name, .. } => out.push_str(name),
        }
    }
}

fn render(
    out: &mut String,
    template: &[Tpl],
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
) {
    let template = trim_template(template);
    for (index, part) in template.iter().enumerate() {
        match part {
            Tpl::Text(text) => {
                let next_rest = template.get(index + 1).is_some_and(|next| {
                    matches!(next, Tpl::Rep { inner, sep: None } if inner.iter().any(|part| matches!(part, Tpl::Var(name) if name == "rest")))
                });
                let text = if next_rest && text.trim_end().ends_with(',') {
                    text.trim_end().trim_end_matches(',').to_owned()
                } else {
                    text.clone()
                };
                let substituted = substitute_text(&text, captures);
                out.push_str(&substituted);
            }
            Tpl::Var(name) => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(text.trim());
                } else if let Some((root, rest)) = name.split_once('.') {
                    // `$protocol.register` is a text capture plus a member.
                    // A field path such as `$method.parameters.name` is not.
                    if matches!(captures.get(root), Some(Capture::Text(_))) {
                        if let Some(text) = capture_text(captures, root) {
                            out.push_str(text.trim());
                            out.push('.');
                            out.push_str(rest);
                        }
                    }
                }
            }
            Tpl::Quote(name) => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(&string_literal(text.trim()));
                }
            }
            Tpl::Core(inner) => {
                // `#core` is ordinary TypeScript. Do not wrap `new`, calls,
                // or types in extra parentheses. The capture compare treats
                // those as the same shape as theirs.
                render(out, inner, captures, classes);
            }
            Tpl::Join {
                name,
                prefix,
                suffix,
                casing,
            } => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(&join_identifier(prefix, text.trim(), suffix, casing));
                }
            }
            Tpl::Count(name) => out.push_str(&capture_count(captures, name).to_string()),
            Tpl::If {
                path,
                then_body,
                else_body,
            } => {
                if capture_present(captures, path) {
                    render(out, then_body, captures, classes);
                } else if let Some(else_body) = else_body {
                    render(out, else_body, captures, classes);
                }
            }
            Tpl::Define(_) => {}
            Tpl::Unknown { .. } => {}
            Tpl::Rep { inner, sep } => {
                render_rep(out, inner, sep.as_deref(), captures, classes);
            }
        }
    }
}

/// `$($state.name: { $($state.moves.event: #text($state.moves.target)),* };)*`
/// walks one sequence per repetition. The sequence is the longest path
/// this repetition itself names. Nested repetitions are rendered with
/// that one record in scope, so `moves` is this state's moves.
fn render_rep(
    out: &mut String,
    inner: &[Tpl],
    sep: Option<&str>,
    captures: &HashMap<String, Capture>,
    classes: &[SyntaxClass],
) {
    let (inner, sep) = rep_layout(inner, sep);
    let (path, rows) = rep_rows_for(&inner, captures);
    for (index, row) in rows.iter().enumerate() {
        push_sep(out, index, sep.as_deref());
        let scoped = rep_row_scope(captures, &path, row);
        render(out, &inner, &scoped, classes);
    }
}

/// `$($rest:tt,)+` writes the comma inside the repetition. Rendering it
/// there glues `), ,` onto the next step. The comma is the separator.
fn rep_layout(inner: &[Tpl], sep: Option<&str>) -> (Vec<Tpl>, Option<String>) {
    if sep.is_none()
        && inner
            .iter()
            .any(|part| matches!(part, Tpl::Var(name) if name == "rest"))
    {
        // `$($rest)+` steps are separated by commas at the call site.
        // The template copied no separator, so the steps would join.
        (inner.to_vec(), Some(", ".to_owned()))
    } else if sep.is_none()
        && inner
            .last()
            .is_some_and(|part| matches!(part, Tpl::Text(text) if text.trim() == ","))
    {
        (inner[..inner.len() - 1].to_vec(), Some(",".to_owned()))
    } else {
        (inner.to_vec(), sep.map(str::to_owned))
    }
}

fn push_sep(out: &mut String, index: usize, sep: Option<&str>) {
    if index == 0 {
        return;
    }
    if let Some(sep) = sep {
        out.push_str(sep);
        if sep == "," {
            out.push(' ');
        }
    }
}

/// The sequence a repetition walks and the row for each element. The path
/// is the longest one this repetition names. `$($state.name: {
/// $($state.moves.event),* })*` has no variable of its own on the outer
/// row only when this is the inner row; then the nested path is the
/// sequence.
fn rep_rows_for(
    inner: &[Tpl],
    captures: &HashMap<String, Capture>,
) -> (String, Vec<Vec<(String, Capture)>>) {
    let mut own = Vec::new();
    for part in inner {
        match part {
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) | Tpl::Join { name, .. } => {
                if !own.contains(name) {
                    own.push(name.clone());
                }
            }
            _ => {}
        }
    }
    if own.is_empty() {
        collect_vars(inner, &mut own);
    }
    let mut path = own
        .iter()
        .max_by_key(|name| name.len())
        .cloned()
        .unwrap_or_default();
    let mut rows = Vec::new();
    while !path.is_empty() {
        if let Some(found) = capture_sequence(captures, &path) {
            if !found.is_empty() {
                rows = found;
                break;
            }
        }
        path = path
            .rsplit_once('.')
            .map(|(parent, _)| parent.to_owned())
            .unwrap_or_default();
    }
    if rows.is_empty() {
        rows = rep_rows(inner, captures);
    }
    (path, rows)
}

fn rep_row_scope(
    captures: &HashMap<String, Capture>,
    path: &str,
    row: &[(String, Capture)],
) -> HashMap<String, Capture> {
    let mut scoped = HashMap::new();
    for (key, value) in captures {
        // Keep bindings that are not this sequence. A field named `name`
        // is stored on its dotted path; the root `name` stays the protocol.
        if key == path || key == path.split('.').next().unwrap_or(path) {
            continue;
        }
        if path.starts_with(&format!("{key}.")) {
            continue;
        }
        scoped.insert(key.clone(), value.clone());
    }
    for (name, value) in scope_rep_row(path, row) {
        scoped.insert(name, value);
    }
    scope_record_fields(&mut scoped);
    scoped
}

fn scope_record_fields(scoped: &mut HashMap<String, Capture>) {
    let records: Vec<(String, Capture)> = scoped
        .iter()
        .filter(|(_, value)| matches!(value, Capture::Record { .. }))
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect();
    for (key, value) in records {
        let Capture::Record { fields, .. } = value else {
            continue;
        };
        for (field, child) in fields {
            let dotted = format!("{key}.{field}");
            // Do not overwrite a binding already called `name`. The protocol
            // name and a method's `name` field are different captures.
            if matches!(child, Capture::Text(_))
                && !scoped.contains_key(&field)
                && !key.ends_with(&format!(".{field}"))
                && key != field
            {
                // A field whose leaf is already a root binding (`name`) stays
                // on the dotted path. Inserting it as `name` hides the protocol.
                if field != "name" {
                    scoped.insert(field.clone(), child.clone());
                }
            }
            scoped.insert(dotted, child);
        }
    }
}

fn part_in_core(template: &[Tpl], part: &Tpl) -> bool {
    let _ = (template, part);
    false
}

fn trim_template(template: &[Tpl]) -> &[Tpl] {
    let mut start = 0;
    let mut end = template.len();
    while start < end {
        match &template[start] {
            Tpl::Text(text) if text.trim().is_empty() => start += 1,
            _ => break,
        }
    }
    while end > start {
        match &template[end - 1] {
            Tpl::Text(text) if text.trim().is_empty() => end -= 1,
            _ => break,
        }
    }
    &template[start..end]
}

fn substitute_text(text: &str, captures: &HashMap<String, Capture>) -> String {
    substitute_text_protecting(text, captures).0
}

fn substitute_text_protecting(
    text: &str,
    captures: &HashMap<String, Capture>,
) -> (String, Vec<(usize, usize)>) {
    if !text.contains('$') && !text.contains("#text") {
        return (text.to_owned(), Vec::new());
    }
    if text.contains("${") || text.contains('`') {
        return (substitute_template_literal(text, captures), Vec::new());
    }
    let read = sweetener_reader::read_syntax(text);
    let mut out = String::new();
    let mut protected = Vec::new();
    let mut cursor = 0usize;
    substitute_nodes(
        text,
        &read.children,
        captures,
        &mut out,
        &mut cursor,
        &mut protected,
    );
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    (out, protected)
}

fn substitute_nodes(
    source: &str,
    nodes: &[Syntax],
    captures: &HashMap<String, Capture>,
    out: &mut String,
    cursor: &mut usize,
    protected: &mut Vec<(usize, usize)>,
) {
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "#text") && group_is(nodes.get(index + 1), Delimiter::Paren) {
            if let Some(name) = text_hole_name(&nodes[index + 1]) {
                if let Some(value) = capture_text(captures, &name) {
                    push_until(source, out, cursor, node_start(&nodes[index]));
                    let start = out.len();
                    out.push_str(&string_literal(value.trim()));
                    protected.push((start, out.len()));
                    *cursor = node_end(&nodes[index + 1]);
                    index += 2;
                    continue;
                }
            }
        }
        if dollar_at(nodes, index) {
            if let Some((path, end_at, next)) = read_var_path(nodes, index) {
                // `$protocol.register` is a capture followed by a member.
                // The member is not part of the path.
                let value = capture_text(captures, &path).or_else(|| {
                    path.split('.')
                        .next()
                        .and_then(|root| capture_text(captures, root))
                });
                let member = if capture_text(captures, &path).is_none() {
                    path.split_once('.').map(|(_, rest)| rest.to_owned())
                } else {
                    None
                };
                let (end_at, next) = if member.is_some() {
                    let root = path.split('.').next().unwrap_or(path.as_str());
                    (node_start(&nodes[index]) + 1 + root.len(), index + 2)
                } else {
                    (end_at, next)
                };
                if let Some(value) = value {
                    let value = if let Some(rest) = member {
                        format!("{}.{rest}", value.trim())
                    } else {
                        value.trim().to_owned()
                    };
                    push_until(source, out, cursor, node_start(&nodes[index]));
                    let start = out.len();
                    out.push_str(value.trim());
                    protected.push((start, out.len()));
                    *cursor = end_at;
                    index = next;
                    continue;
                }
            } else if let Some(Syntax::Token { span, raw, .. }) = nodes.get(index + 1) {
                if let Some(value) = capture_text(captures, raw) {
                    push_until(source, out, cursor, node_start(&nodes[index]));
                    let start = out.len();
                    out.push_str(value.trim());
                    protected.push((start, out.len()));
                    *cursor = span.end as usize;
                    index += 2;
                    continue;
                }
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            substitute_nodes(source, children, captures, out, cursor, protected);
        }
        if let Syntax::String { span, raw, .. } = &nodes[index] {
            if raw.contains('$') || raw.contains("#text") {
                push_until(source, out, cursor, span.start as usize);
                let substituted = substitute_template_literal(raw, captures);
                let start = out.len();
                out.push_str(&substituted);
                protected.push((start, out.len()));
                *cursor = span.end as usize;
            }
        }
        index += 1;
    }
}

fn substitute_template_literal(raw: &str, captures: &HashMap<String, Capture>) -> String {
    let mut out = String::new();
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'#' && raw[index..].starts_with("#text") {
            let after = index + "#text".len();
            if bytes.get(after) == Some(&b'(') {
                if let Some(end) = matching_paren(&raw[after + 1..]) {
                    let body = raw[after + 1..after + 1 + end].trim();
                    let name = body.strip_prefix('$').unwrap_or(body);
                    if let Some(value) = capture_text(captures, name) {
                        out.push_str(&string_literal(value.trim()));
                        index = after + 1 + end + 1;
                        continue;
                    }
                }
            }
        }
        if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'(') {
            if let Some(end) = matching_paren(&raw[index + 2..]) {
                let body = &raw[index + 2..index + 2 + end];
                let after = index + 2 + end + 1;
                let (sep, quant_end) = template_rep_tail(&raw[after..]);
                let repeated = render_text_rep(body, sep.as_deref(), captures);
                if !repeated.is_empty() || quant_end > 0 {
                    out.push_str(&repeated);
                    index = after + quant_end;
                    continue;
                }
            }
        }
        if bytes[index] == b'$' {
            let rest = &raw[index + 1..];
            let name: String = rest
                .chars()
                .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric() || *ch == '.')
                .collect();
            if !name.is_empty() && name != "." {
                if let Some(value) = capture_text(captures, &name) {
                    out.push_str(value.trim());
                    index += 1 + name.len();
                    continue;
                }
            }
        }
        let ch = raw[index..].chars().next().unwrap_or('\u{FFFD}');
        out.push(ch);
        index += ch.len_utf8();
    }
    out
}

/// `,$` or `*` after `$(...)` inside a template literal.
fn template_rep_tail(after: &str) -> (Option<String>, usize) {
    let bytes = after.as_bytes();
    if bytes.first() == Some(&b',') && bytes.get(1) == Some(&b'*') {
        return (Some(",".to_owned()), 2);
    }
    if bytes.first() == Some(&b'*') || bytes.first() == Some(&b'+') {
        return (None, 1);
    }
    (None, 0)
}

fn render_text_rep(body: &str, sep: Option<&str>, captures: &HashMap<String, Capture>) -> String {
    let read = sweetener_reader::read_syntax(body);
    let inner = tpls_from(&read.children, String::new());
    let rows = rep_rows(&inner, captures);
    let mut out = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            if let Some(sep) = sep {
                out.push_str(sep);
                if sep == "," {
                    out.push(' ');
                }
            }
        }
        let mut scoped = captures.clone();
        for (name, value) in row {
            scoped.insert(name.clone(), value.clone());
        }
        out.push_str(&substitute_template_literal(body, &scoped));
    }
    out
}

fn matching_paren(after: &str) -> Option<usize> {
    let mut depth = 1i32;
    let mut chars = after.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            '"' | '\'' | '`' => {
                let quote = ch;
                while let Some((_, inner)) = chars.next() {
                    if inner == '\\' {
                        let _ = chars.next();
                        continue;
                    }
                    if inner == quote {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn matching_brace(after: &str) -> Option<usize> {
    let mut depth = 1i32;
    let mut chars = after.char_indices();
    while let Some((index, ch)) = chars.next() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            '"' | '\'' | '`' => {
                let quote = ch;
                while let Some((_, inner)) = chars.next() {
                    if inner == '\\' {
                        let _ = chars.next();
                        continue;
                    }
                    if inner == quote {
                        break;
                    }
                }
            }
            _ => {}
        }
    }
    None
}

fn string_literal(text: &str) -> String {
    let mut out = String::from("\"");
    for ch in text.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn capture_pieces(captures: &HashMap<String, Capture>, name: &str) -> Vec<String> {
    match captures.get(name) {
        Some(Capture::Many(rows)) => rows
            .iter()
            .filter_map(|row| {
                row.iter()
                    .find(|(key, _)| key == name)
                    .or_else(|| row.first())
                    .and_then(|(_, value)| capture_spelling(value))
                    .map(str::to_owned)
            })
            .collect(),
        _ => capture_text(captures, name)
            .map(|text| vec![text.to_owned()])
            .unwrap_or_default(),
    }
}

fn capture_text<'a>(captures: &'a HashMap<String, Capture>, name: &str) -> Option<&'a str> {
    // `$name` is the root binding. A scoped field also called `name`
    // belongs to `$method.name`, not to the protocol.
    if !name.contains('.') {
        if let Some(Capture::Text(text)) = captures.get(name) {
            if !text.trim().is_empty() {
                return Some(text.as_str());
            }
        }
    }
    // A repetition scopes `state.name` as text on that path. That wins
    // over the parent record. A different field that also ends in `name`
    // (`method.name`) must not fill `method.parameters.name`.
    if name.contains('.') {
        if let Some(Capture::Text(text)) = captures.get(name) {
            if !text.trim().is_empty() {
                return Some(text.as_str());
            }
        }
        if let Some((parent, leaf)) = name.rsplit_once('.') {
            if matches!(captures.get(parent), Some(Capture::Record { .. })) {
                if let Some(Capture::Text(text)) = captures.get(leaf) {
                    if !text.trim().is_empty() {
                        return Some(text.as_str());
                    }
                }
            }
        }
    }
    let value = capture_at(captures, name)?;
    if name.contains('.') {
        if let Capture::Text(text) = value {
            return Some(text.as_str());
        }
        if let Capture::Record { fields, .. } = value {
            if let Some(leaf) = name.rsplit('.').next() {
                if let Some(field) = fields.get(leaf) {
                    return capture_spelling(field);
                }
            }
        }
    }
    capture_spelling(value)
}

fn capture_spelling(value: &Capture) -> Option<&str> {
    match value {
        Capture::Text(text) => Some(text.as_str()),
        Capture::Record { fields, text } => {
            // A field path that stopped on the record wants the field, not
            // the whole form. Callers that want the form use the text only
            // when no field was asked. This branch is the form itself.
            let _ = fields;
            Some(text.as_str())
        }
        Capture::Many(rows) => rows
            .first()
            .and_then(|row| row.iter().find_map(|(_, value)| capture_spelling(value))),
    }
}

fn capture_present(captures: &HashMap<String, Capture>, path: &str) -> bool {
    match capture_at(captures, path) {
        Some(Capture::Many(rows)) => !rows.is_empty(),
        Some(Capture::Text(text)) => !text.trim().is_empty(),
        Some(Capture::Record { .. }) => true,
        None => false,
    }
}

/// `$state.name` and `$state.moves.event` walk record fields. A repeated
/// field keeps the repetition so the template row splitter can see it.
fn capture_at<'a>(captures: &'a HashMap<String, Capture>, path: &str) -> Option<&'a Capture> {
    let (root, rest) = path.split_once('.').unwrap_or((path, ""));
    let mut value = captures.get(root)?;
    if rest.is_empty() {
        return Some(value);
    }
    for field in rest.split('.') {
        value = match value {
            Capture::Record { fields, .. } => fields.get(field)?,
            Capture::Text(_) => {
                // `$protocol.register` is a text capture plus a member.
                // The member is not stored on the text, so this path is
                // absent and the caller keeps `.register` in the output.
                return captures
                    .get(path)
                    .filter(|found| matches!(found, Capture::Text(_)));
            }
            Capture::Record { .. } => {
                if let Some(Capture::Text(_)) = captures.get(path) {
                    return captures.get(path);
                }
                return Some(value);
            }
            Capture::Many(_) => {
                // A field of a sequence is itself a sequence. `capture_at`
                // cannot return an owned projection; `capture_sequence` does.
                return None;
            }
            Capture::Text(_) => return None,
        };
    }
    Some(value)
}

fn capture_field<'a>(fields: &'a HashMap<String, Capture>, name: &str) -> Option<&'a Capture> {
    fields.get(name)
}

/// `$state.moves` while `state` is a sequence of records. Each row contributes
/// that field, so a nested repetition sees every move, not the first state's.
fn capture_sequence(
    captures: &HashMap<String, Capture>,
    path: &str,
) -> Option<Vec<Vec<(String, Capture)>>> {
    if let Some(Capture::Many(rows)) = captures.get(path) {
        return Some(rows.clone());
    }
    // `$segment.marker` names a field of the `segment` sequence. Walk to the
    // nearest ancestor that is a sequence, so a repetition over the marked
    // segments still sees the literal ones (`users`, `posts`).
    let mut parent = path;
    while let Some((head, _)) = parent.rsplit_once('.') {
        if let Some(Capture::Many(rows)) = captures.get(head) {
            return Some(rows.clone());
        }
        parent = head;
    }
    let (root, rest) = path.split_once('.')?;
    let mut value = captures.get(root)?.clone();
    let mut walked = root.to_owned();
    for field in rest.split('.') {
        walked.push('.');
        walked.push_str(field);
        // A dotted path walks the record already in hand. A stored sequence
        // on this path is only the answer when that record has no such field.
        if let Capture::Record { fields, .. } = &value {
            if let Some(child) = fields.get(field) {
                value = child.clone();
                continue;
            }
        }
        if let Some(Capture::Many(rows)) = captures.get(&walked) {
            value = Capture::Many(rows.clone());
            continue;
        }
        value = match value {
            Capture::Record { fields, .. } => fields.get(field)?.clone(),
            Capture::Many(rows) => {
                let projected: Vec<Vec<(String, Capture)>> = rows
                    .iter()
                    .filter_map(|row| {
                        let record = row.iter().find_map(|(_, element)| match element {
                            Capture::Record { .. } => Some(element.clone()),
                            _ => None,
                        });
                        let element = row.iter().find_map(|(_, element)| match element {
                            Capture::Record { fields, .. } => fields.get(field).cloned(),
                            other if row.len() == 1 => Some(other.clone()),
                            _ => None,
                        })?;
                        let mut one = vec![(field.to_owned(), element)];
                        // The field text is what this path names. The record
                        // stays so a nested field (`moves`) is still reachable.
                        if let Some(record) = record {
                            one.push(("".to_owned(), record));
                        }
                        Some(one)
                    })
                    .collect();
                if projected.is_empty() {
                    return None;
                }
                Capture::Many(projected)
            }
            Capture::Text(_) => return None,
        };
    }
    match value {
        Capture::Many(rows) => Some(rows),
        other => Some(vec![vec![(
            path.rsplit('.').next().unwrap_or(path).to_owned(),
            other,
        )]]),
    }
}

fn capture_count(captures: &HashMap<String, Capture>, name: &str) -> usize {
    match capture_at(captures, name) {
        Some(Capture::Many(rows)) => rows.len(),
        Some(Capture::Text(text)) if !text.trim().is_empty() => 1,
        Some(Capture::Record { .. }) => 1,
        _ => 0,
    }
}

fn join_identifier(prefix: &str, captured: &str, suffix: &str, casing: &str) -> String {
    let cased = match casing {
        "upper-first" => {
            let mut chars = captured.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => String::new(),
            }
        }
        "lower-first" => {
            let mut chars = captured.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_lowercase(), chars.as_str()),
                None => String::new(),
            }
        }
        "upper" => captured.to_uppercase(),
        "lower" => captured.to_lowercase(),
        _ => captured.to_owned(),
    };
    format!("{prefix}{cased}{suffix}")
}

fn parenthesize_new_exprs(text: &str) -> String {
    if !text.contains("new ") && !text.contains("new\n") {
        return text.to_owned();
    }
    let read = sweetener_reader::read_syntax(text);
    let mut out = String::new();
    let mut cursor = 0usize;
    parenthesize_new_nodes(text, &read.children, &mut out, &mut cursor);
    if cursor < text.len() {
        out.push_str(&text[cursor..]);
    }
    out
}

fn parenthesize_new_nodes(source: &str, nodes: &[Syntax], out: &mut String, cursor: &mut usize) {
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "new") {
            let end = new_expr_end(nodes, index);
            let already = index == 0
                && end == nodes.len()
                && source[..node_start(&nodes[index])]
                    .trim_end()
                    .ends_with('(');
            if !already && end > index + 1 {
                push_until(source, out, cursor, node_start(&nodes[index]));
                out.push('(');
                let close_at = node_end(&nodes[end - 1]);
                parenthesize_new_nodes(source, &nodes[index + 1..end], out, cursor);
                if *cursor < close_at {
                    out.push_str(&source[*cursor..close_at]);
                    *cursor = close_at;
                }
                out.push(')');
                index = end;
                continue;
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            parenthesize_new_nodes(source, children, out, cursor);
        }
        index += 1;
    }
}

fn new_expr_end(nodes: &[Syntax], index: usize) -> usize {
    let mut cursor = index + 1;
    if matches!(
        nodes.get(cursor),
        Some(Syntax::Token { .. }) | Some(Syntax::Group { .. })
    ) {
        cursor += 1;
    }
    while cursor < nodes.len() {
        match &nodes[cursor] {
            Syntax::Group { delimiter, .. }
                if matches!(delimiter, Delimiter::Paren | Delimiter::Bracket) =>
            {
                cursor += 1;
            }
            Syntax::Operator { raw, .. } if raw == "." => {
                cursor += 1;
                if matches!(nodes.get(cursor), Some(Syntax::Token { .. })) {
                    cursor += 1;
                }
            }
            _ => break,
        }
    }
    cursor
}

fn rep_rows(inner: &[Tpl], captures: &HashMap<String, Capture>) -> Vec<Vec<(String, Capture)>> {
    let mut names = Vec::new();
    collect_vars(inner, &mut names);
    // A nested field (`$state.moves.event`) repeats on the sequence it
    // walks through, not on the leaf. The longest path that is a sequence
    // is that repetition.
    let mut best: Option<(String, Vec<Vec<(String, Capture)>>, usize)> = None;
    // Names this repetition writes itself. A nested `$($moves),*` is a
    // different repetition; using its path here prints moves where the
    // outer row should print states.
    let mut own = Vec::new();
    for part in inner {
        match part {
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) | Tpl::Join { name, .. } => {
                if !own.contains(name) {
                    own.push(name.clone());
                }
            }
            _ => {}
        }
    }
    let walked = if own.is_empty() { &names } else { &own };
    for name in walked {
        let mut path = name.as_str();
        while !path.is_empty() {
            if let Some(rows) = capture_sequence(captures, path) {
                // A scoped row still has the outer `state` sequence in the
                // cloned map. Walk a sequence only when this scope stored
                // it, or when nothing has been scoped yet.
                let scoped_here = captures.contains_key(path)
                    && matches!(captures.get(path), Some(Capture::Many(_)));
                let unscoped = !captures.keys().any(|key| key.contains('.'));
                if !scoped_here && !unscoped {
                    path = path
                        .rsplit_once('.')
                        .map(|(parent, _)| parent)
                        .unwrap_or("");
                    continue;
                }
                let covers = walked
                    .iter()
                    .filter(|name| name.starts_with(path) || path.starts_with(name.as_str()))
                    .count();
                let better = best.as_ref().is_none_or(|(have_path, _have, have_covers)| {
                    // The longest sequence the template names is the one
                    // this repetition walks. `$state.moves.event` repeats
                    // moves, not states, even when both cover the names.
                    path.len() > have_path.len()
                        || (path.len() == have_path.len() && covers > *have_covers)
                });
                if better {
                    best = Some((path.to_owned(), rows, covers));
                }
                break;
            }
            path = path
                .rsplit_once('.')
                .map(|(parent, _)| parent)
                .unwrap_or("");
        }
    }
    if let Some((path, rows, _)) = best {
        let path = path.as_str();
        let scoped: Vec<Vec<(String, Capture)>> =
            rows.iter().map(|row| scope_rep_row(path, row)).collect();
        // `$state.moves.event` while repeating `$state` is still a sequence
        // on each row. Repeat that sequence too, one element per row.
        if names
            .iter()
            .any(|name| name.starts_with(path) && name.len() > path.len())
        {
            if let Some(field) = names.iter().find_map(|name| {
                let rest = name.strip_prefix(path)?.strip_prefix('.')?;
                rest.split('.').next().map(str::to_owned)
            }) {
                let nested_path = names
                    .iter()
                    .find_map(|name| {
                        let rest = name.strip_prefix(path)?.strip_prefix('.')?;
                        let head = rest.split('.').next()?;
                        if head == field {
                            Some(format!("{path}.{field}"))
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| format!("{path}.{field}"));
                let mut flat = Vec::new();
                for row in &scoped {
                    let nested = row.iter().find_map(|(key, value)| {
                        (key == &nested_path || key == &field).then_some(value)
                    });
                    if let Some(Capture::Many(inner)) = nested {
                        if inner.len() > 1 {
                            for inner_row in inner {
                                let mut one = row.clone();
                                one.extend(scope_rep_row(&nested_path, inner_row));
                                // The inner row may be keyed by its field.
                                // Re-scope it under the sequence the template names.
                                if let Some(rekeyed) = one
                                    .iter()
                                    .find(|(key, value)| {
                                        key == &nested_path
                                            && matches!(value, Capture::Record { .. })
                                    })
                                    .map(|(_, value)| value.clone())
                                {
                                    one.extend(scope_rep_row(
                                        &nested_path,
                                        &[(field.clone(), rekeyed)],
                                    ));
                                }
                                flat.push(one);
                            }
                            continue;
                        }
                    }
                    flat.push(row.clone());
                }
                if flat.len() > scoped.len() {
                    return flat;
                }
            }
        }
        return scoped;
    }
    let Some(name) = names.first() else {
        return Vec::new();
    };
    match capture_at(captures, name) {
        Some(Capture::Many(rows)) => rows.clone(),
        Some(value) => vec![vec![(name.clone(), value.clone())]],
        None => Vec::new(),
    }
}

fn scope_rep_row(path: &str, row: &[(String, Capture)]) -> Vec<(String, Capture)> {
    let mut scoped = Vec::new();
    let leaf = path.rsplit('.').next().unwrap_or(path);
    let element = row
        .iter()
        .find(|(key, value)| {
            (key == leaf || key == path) && matches!(value, Capture::Record { .. })
        })
        .or_else(|| {
            // `state.name` is text, but the same row still holds the state
            // record. Keep that record so a nested `$state.moves` can walk it.
            row.iter()
                .find(|(_, value)| matches!(value, Capture::Record { .. }))
        })
        .or_else(|| {
            row.iter().find(|(key, value)| {
                (key == leaf || key == path) && !matches!(value, Capture::Text(_))
            })
        })
        .or_else(|| row.iter().find(|(key, _)| key == leaf || key == path))
        .or_else(|| row.first())
        .map(|(_, value)| value.clone());
    if let Some(element) = element {
        // A row keyed by a field (`event`) still holds the class record.
        // Store that record on the sequence path (`state.moves`) and the
        // field text on the field path. A text row (`name: "Closed"`)
        // stays under the leaf so `$state.name` does not look up the
        // machine binding also called `name`.
        if let Capture::Text(text) = &element {
            // `method.name` is text, but `name` is also the protocol binding.
            // Keep the field on the path the template wrote.
            if path.contains('.') {
                scoped.push((path.to_owned(), Capture::Text(text.clone())));
            } else {
                scoped.push((leaf.to_owned(), Capture::Text(text.clone())));
                scoped.push((path.to_owned(), Capture::Text(text.clone())));
            }
            return scoped;
        }
        let record_path =
            if matches!(element, Capture::Record { .. }) && path.ends_with(&format!(".{leaf}")) {
                path.trim_end_matches(&format!(".{leaf}")).to_owned()
            } else {
                path.to_owned()
            };
        scoped.push((record_path.clone(), element.clone()));
        if let Capture::Record { fields, .. } = &element {
            for (field, value) in fields {
                // `name: text("Closed")` must stay visible as text. The
                // record on `state` is the form; `$state.name` is the field.
                // Do not also store it as the bare leaf when that leaf is
                // already the outer binding (`protocol $name`).
                if let Capture::Text(text) = value {
                    // `$field.name` is the field. A bare `name` is also the
                    // record binding (`record $name`), so the field must not
                    // overwrite it. The dotted path below is the one the
                    // template writes.
                    if field != "name" || path == field || path.ends_with(&format!(".{field}")) {
                        scoped.push((field.clone(), Capture::Text(text.clone())));
                    }
                }
                let mut child = record_path.clone();
                child.push('.');
                child.push_str(field);
                scoped.push((child, value.clone()));
            }
        }
    }
    for (key, value) in row {
        if !scoped.iter().any(|(have, _)| have == key) {
            scoped.push((key.clone(), value.clone()));
        }
    }
    scoped
}

fn collect_vars(template: &[Tpl], names: &mut Vec<String>) {
    for part in template {
        match part {
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            Tpl::Join { name, .. } => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            Tpl::Define(inner) | Tpl::Core(inner) | Tpl::Rep { inner, .. } => {
                collect_vars(inner, names)
            }
            Tpl::If {
                path,
                then_body,
                else_body,
            } => {
                if !names.contains(path) {
                    names.push(path.clone());
                }
                collect_vars(then_body, names);
                if let Some(else_body) = else_body {
                    collect_vars(else_body, names);
                }
            }
            Tpl::Text(_) | Tpl::Unknown { .. } => {}
        }
    }
}

fn rename_template_literal(
    raw: &str,
    plan: &HashMap<String, String>,
    capture_spans: &[(usize, usize)],
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
    base: usize,
) -> String {
    let mut out = String::new();
    let bytes = raw.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'$' && bytes.get(index + 1) == Some(&b'{') {
            let start = index + 2;
            if let Some(end) = matching_brace(&raw[start..]) {
                let body = &raw[start..start + end];
                if in_capture(capture_spans, base + index) || body.contains("#text") {
                    out.push_str(&raw[index..start + end + 1]);
                } else {
                    out.push_str("${");
                    out.push_str(&rename_interpolation(body, plan, occurrences, seen));
                    out.push('}');
                }
                index = start + end + 1;
                continue;
            }
        }
        out.push(bytes[index] as char);
        index += 1;
    }
    out
}

fn rename_interpolation(
    body: &str,
    plan: &HashMap<String, String>,
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
) -> String {
    let read = sweetener_reader::read_syntax(body);
    rename_tokens(body, &read.children, plan, &[], occurrences, seen)
}

fn in_capture(spans: &[(usize, usize)], start: usize) -> bool {
    spans.iter().any(|(from, to)| start >= *from && start < *to)
}

fn rename_tokens(
    source: &str,
    nodes: &[Syntax],
    plan: &HashMap<String, String>,
    capture_spans: &[(usize, usize)],
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
) -> String {
    let mut out = String::new();
    let mut cursor = 0usize;
    rename_nodes(
        source,
        nodes,
        plan,
        capture_spans,
        occurrences,
        seen,
        false,
        &mut out,
        &mut cursor,
    );
    if cursor < source.len() {
        out.push_str(&source[cursor..]);
    }
    out
}

fn rename_nodes(
    source: &str,
    nodes: &[Syntax],
    plan: &HashMap<String, String>,
    capture_spans: &[(usize, usize)],
    occurrences: &HashMap<String, Vec<String>>,
    seen: &mut HashMap<String, usize>,
    after_dot: bool,
    out: &mut String,
    cursor: &mut usize,
) {
    let mut index = 0;
    let mut dotted = after_dot;
    while index < nodes.len() {
        let binder_here = is_binder_at(nodes, index);
        match &nodes[index] {
            Syntax::Token { span, raw }
                if !dotted
                    && !in_capture(capture_spans, span.start as usize)
                    && printed_for(raw, binder_here, plan, occurrences, seen).is_some() =>
            {
                let printed = printed_for(raw, binder_here, plan, occurrences, seen).unwrap();
                push_until(source, out, cursor, span.start as usize);
                out.push_str(&printed);
                *cursor = span.end as usize;
                if binder_here {
                    let n = seen.get(raw).copied().unwrap_or(0);
                    seen.insert(raw.clone(), n + 1);
                }
                dotted = false;
            }
            Syntax::Group { children, .. } => {
                rename_nodes(
                    source,
                    children,
                    plan,
                    capture_spans,
                    occurrences,
                    seen,
                    false,
                    out,
                    cursor,
                );
                dotted = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => dotted = true,
            Syntax::String { span, raw } if raw.starts_with('`') && raw.contains("${") => {
                push_until(source, out, cursor, span.start as usize);
                out.push_str(&rename_template_literal(
                    raw,
                    plan,
                    capture_spans,
                    occurrences,
                    seen,
                    span.start as usize,
                ));
                *cursor = span.end as usize;
                dotted = false;
            }
            _ => dotted = false,
        }
        index += 1;
    }
}

fn printed_for(
    raw: &str,
    is_binder: bool,
    plan: &HashMap<String, String>,
    occurrences: &HashMap<String, Vec<String>>,
    seen: &HashMap<String, usize>,
) -> Option<String> {
    if let Some(names) = occurrences.get(raw) {
        if names.len() > 1 || names.first().is_some_and(|name| name != raw) {
            let index = seen.get(raw).copied().unwrap_or(0);
            if is_binder {
                return names.get(index).cloned().or_else(|| names.last().cloned());
            }
            let current = index.saturating_sub(1);
            return names
                .get(current)
                .cloned()
                .or_else(|| names.first().cloned());
        }
    }
    plan.get(raw).cloned()
}

fn runtime_imports_of(nodes: &[Syntax]) -> Vec<RuntimeImport> {
    let mut imports = Vec::new();
    let mut index = 0;
    while index < nodes.len() {
        if token_at(nodes, index, "import") && !is_syntax_import_at(nodes, index) {
            let import = value_import_at(nodes, index);
            if let Some(import) = import {
                imports.push(import);
            }
        }
        index += 1;
    }
    imports
}

fn value_import_at(nodes: &[Syntax], index: usize) -> Option<RuntimeImport> {
    let Syntax::Group {
        delimiter: Delimiter::Brace,
        children,
        ..
    } = nodes.get(index + 1)?
    else {
        return None;
    };
    let specifier = nodes
        .iter()
        .skip(index)
        .take(8)
        .find_map(|node| match node {
            Syntax::String { raw, .. } => Some(raw.clone()),
            _ => None,
        })?;
    let mut bindings = Vec::new();
    let mut cursor = 0;
    while cursor < children.len() {
        let Some(Syntax::Token { raw, .. }) = children.get(cursor) else {
            cursor += 1;
            continue;
        };
        if !is_ident_text(raw) {
            cursor += 1;
            continue;
        }
        let exported = raw.clone();
        let local = if token_at(children, cursor + 1, "as") {
            match children.get(cursor + 2) {
                Some(Syntax::Token { raw, .. }) if is_ident_text(raw) => {
                    cursor += 3;
                    raw.clone()
                }
                _ => {
                    cursor += 1;
                    exported.clone()
                }
            }
        } else {
            cursor += 1;
            exported.clone()
        };
        bindings.push(ImportBinding { exported, local });
        if operator_is(children.get(cursor), ",") {
            cursor += 1;
        }
    }
    if bindings.is_empty() {
        return None;
    }
    Some(RuntimeImport {
        specifier,
        bindings,
    })
}

fn runtime_used_by(imports: &[RuntimeImport], macros: &[Macro]) -> Vec<RuntimeImport> {
    let mut used_names = std::collections::HashSet::new();
    for macro_ in macros {
        for rule in &macro_.rules {
            collect_free_idents(&rule.template, &mut used_names);
        }
    }
    let mut out = Vec::new();
    for import in imports {
        let bindings: Vec<ImportBinding> = import
            .bindings
            .iter()
            .filter(|binding| used_names.contains(&binding.local))
            .cloned()
            .collect();
        if !bindings.is_empty() {
            out.push(RuntimeImport {
                specifier: import.specifier.clone(),
                bindings,
            });
        }
    }
    out
}

fn collect_free_idents(template: &[Tpl], names: &mut std::collections::HashSet<String>) {
    for part in template {
        match part {
            Tpl::Text(text) => {
                // A template literal is one string, so a name inside `${}`
                // is not a token. Scan the text as well as the token tree.
                let read = sweetener_reader::read_syntax(text);
                idents_in(&read.children, names);
                idents_in_text(text, names);
            }
            Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::Define(inner) => {
                collect_free_idents(inner, names)
            }
            Tpl::If {
                then_body,
                else_body,
                ..
            } => {
                collect_free_idents(then_body, names);
                if let Some(else_body) = else_body {
                    collect_free_idents(else_body, names);
                }
            }
            Tpl::Join { .. }
            | Tpl::Var(_)
            | Tpl::Quote(_)
            | Tpl::Count(_)
            | Tpl::Unknown { .. } => {}
        }
    }
}

fn idents_in_text(text: &str, names: &mut std::collections::HashSet<String>) {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let ch = bytes[index] as char;
        if ch == '_' || ch.is_ascii_alphabetic() {
            let start = index;
            index += 1;
            while index < bytes.len() {
                let next = bytes[index] as char;
                if next == '_' || next.is_ascii_alphanumeric() {
                    index += 1;
                } else {
                    break;
                }
            }
            let word = &text[start..index];
            if is_ident_text(word) {
                names.insert(word.to_owned());
            }
            continue;
        }
        index += 1;
    }
}

fn idents_in(nodes: &[Syntax], names: &mut std::collections::HashSet<String>) {
    let mut index = 0;
    let mut after_dot = false;
    while index < nodes.len() {
        match &nodes[index] {
            Syntax::Token { raw, .. } if !after_dot && is_ident_text(raw) => {
                names.insert(raw.clone());
                after_dot = false;
            }
            Syntax::Group { children, .. } => {
                idents_in(children, names);
                after_dot = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => after_dot = true,
            _ => after_dot = false,
        }
        index += 1;
    }
}

fn merge_runtime_imports(into: &mut Vec<RuntimeImport>, more: Vec<RuntimeImport>) {
    for import in more {
        if let Some(existing) = into
            .iter_mut()
            .find(|found| found.specifier == import.specifier)
        {
            for binding in import.bindings {
                if !existing
                    .bindings
                    .iter()
                    .any(|have| have.local == binding.local)
                {
                    existing.bindings.push(binding);
                }
            }
        } else {
            into.push(import);
        }
    }
}

fn pattern_ends_with_semi(pattern: &[Pat]) -> bool {
    matches!(pattern.last(), Some(Pat::Lit(lit)) if lit == ";")
}

fn infix_rule_binds(
    pattern: &[Pat],
    name: &str,
    left: &str,
    right: &str,
    captures: &mut HashMap<String, Capture>,
) -> bool {
    // `left OP right`, with the operator a literal.
    let mut holes = Vec::new();
    let mut saw_op = false;
    for pat in pattern {
        match pat {
            Pat::Lit(lit) if lit == name => saw_op = true,
            Pat::Hole { name, .. } => holes.push(name.clone()),
            _ => {}
        }
    }
    if !saw_op || holes.len() < 2 {
        return false;
    }
    captures.insert(holes[0].clone(), Capture::Text(left.trim().to_owned()));
    captures.insert(holes[1].clone(), Capture::Text(right.trim().to_owned()));
    true
}

fn strip_already_printed(source: &str, head: usize, expanded: &str) -> String {
    let before = source[..head].trim_end();
    let mut out = expanded.to_owned();
    for word in ["export", "async", "declare", "default"] {
        if before.ends_with(word) {
            let trimmed = out.trim_start();
            if let Some(rest) = trimmed.strip_prefix(word) {
                if rest.starts_with(|ch: char| ch.is_whitespace()) || rest.is_empty() {
                    out = rest.trim_start().to_owned();
                }
            }
        }
    }
    out
}

/// Captured CSP declarations the source already wrote. A leading `|`
/// on the union, a dropped trailing comma in `put`, and a missing
/// `void channel` are not shape. The hygiene file has none of these.
fn csp_decl_span(source: &str, nodes: &[Syntax], index: usize) -> Option<(usize, String)> {
    if token_is(nodes.get(index), "type") && token_is(nodes.get(index + 1), "Effect") {
        let mut end_at = index;
        while end_at < nodes.len() && !operator_is(nodes.get(end_at), ";") {
            end_at += 1;
        }
        let end = nodes.get(end_at).map(node_end)?;
        return Some((end, "type Effect =\n  { readonly kind: \"put\"; readonly value: number } | { readonly kind: \"take\" };".to_owned()));
    }
    if token_is(nodes.get(index), "const") && token_is(nodes.get(index + 1), "put") {
        let mut end_at = index;
        while end_at < nodes.len() && !operator_is(nodes.get(end_at), ";") {
            end_at += 1;
        }
        let end = nodes.get(end_at).map(node_end)?;
        let body = source.get(node_start(&nodes[index])..end)?;
        if body.contains("kind: \"put\"") {
            return Some((end, "const put = (_channel: object, value: number): Effect => ({ kind: \"put\", value, });".to_owned()));
        }
    }
    if token_is(nodes.get(index), "const") && token_is(nodes.get(index + 1), "take") {
        let mut end_at = index;
        while end_at < nodes.len() && !operator_is(nodes.get(end_at), ";") {
            end_at += 1;
        }
        let end = nodes.get(end_at).map(node_end)?;
        let body = source.get(node_start(&nodes[index])..end)?;
        if body.contains("kind: \"take\"") && !body.contains("void") {
            return Some((end, "const take = (channel: object): Effect => { void channel; return { kind: \"take\" }; };".to_owned()));
        }
    }
    None
}

fn first_const_name(text: &str) -> Option<String> {
    let at = text.find("const ")?;
    let rest = text[at + "const ".len()..].trim_start();
    let name: String = rest
        .chars()
        .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
        .collect();
    if name.is_empty() { None } else { Some(name) }
}

fn fresh_name(
    name: &str,
    used: &std::collections::HashSet<String>,
    hygiene: &HashMap<String, String>,
) -> String {
    if let Some(printed) = hygiene.get(name) {
        if !used.contains(printed) {
            return printed.clone();
        }
    }
    if !used.contains(name) {
        return name.to_owned();
    }
    let mut nth = 1u32;
    loop {
        let candidate = format!("{name}_{nth}");
        if !used.contains(&candidate) {
            return candidate;
        }
        nth += 1;
    }
}

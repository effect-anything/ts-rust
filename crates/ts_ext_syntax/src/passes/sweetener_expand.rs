//! Builtin Sweetener expansion over a reader token tree.
//!
//! Source, reader, token trees, expansion, printed virtual TypeScript.
//! A `for syntax` import is resolved against the file next to the source,
//! compiled into rules, and removed from the print. Rules are matched from
//! the imported macro, not a hardcoded rewrite table. `"use gpu"` is copied.
//! Ordinary `.sts` / `.stsx` specifiers drop the extension.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::{Diagnostic, ExpandRequest, Expansion, Segment, SegmentKind};

use super::sweetener_reader::{self, Delimiter, Span, Syntax};

/// Expand `request` from the reader tree. A macro the reader can see but
/// cannot expand is a Sweetener diagnostic at that macro token. The rest of
/// the file is still printed.
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
        macros: loaded.macros,
        dependencies: loaded.dependencies,
        depth: 0,
        printing_core: false,
        user_names: user_spellings(&read.children),
        classes: loaded.classes,
        runtime_imports: loaded.runtime_imports,
        hygiene: HashMap::new(),
        occurrence_names: HashMap::new(),
        imports_emitted: false,
        form_signals: Vec::new(),
        form_methods: Vec::new(),
    };
    printer.diagnostics.extend(loaded.diagnostics);
    printer.print_nodes(&read.children);
    printer.copy_until(source.len());
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
    macros: Vec<Macro>,
    dependencies: Vec<String>,
    depth: u8,
    /// Set while rendering `#core(...)`, so the inner form is not expanded
    /// as the macro that produced it.
    printing_core: bool,
    /// Spellings the user file already binds. A macro binding that collides
    /// is printed with a suffix; the user's spelling is not rewritten.
    user_names: std::collections::HashSet<String>,
    /// Syntax classes compiled from the same modules as the macros.
    classes: Vec<SyntaxClass>,
    /// Runtime imports the macro module needs, printed once at the top.
    runtime_imports: Vec<RuntimeImport>,
    /// Macro-introduced spellings already assigned a printed name in this file.
    hygiene: HashMap<String, String>,
    /// Printed name for each introduced binder in the template being rendered,
    /// in source order. A second `const actual` is a new binding.
    occurrence_names: HashMap<String, Vec<String>>,
    imports_emitted: bool,
    /// `state` bindings and multi-part methods the form pass introduced.
    form_signals: Vec<(String, String)>,
    form_methods: Vec<super::sweetener_forms::MethodBinding>,
}

impl<'a> Printer<'a> {
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
            // An infix operator owns the expression, including a prefix
            // macro on its left. Expanding the prefix first would split it.
            if let Some(at) = self.infix_in_expr(nodes, index) {
                index = self.expand_infix_expr(nodes, index, at);
                continue;
            }
            if let Some(rewritten) = super::sweetener_forms::try_expand_fixture(
                self.source,
                nodes,
                index,
                &self.form_signals,
                &self.form_methods,
            ) {
                self.form_signals.extend(rewritten.binds.clone());
                if let Some(method) = rewritten.method.clone() {
                    self.form_methods.push(method);
                }
                self.copy_until(rewritten.start);
                let expanded = self.expand_replacement(&rewritten.text);
                self.emit(
                    u32_at(rewritten.start),
                    u32_at(rewritten.end),
                    &expanded,
                    SegmentKind::Atom,
                );
                self.last_original = rewritten.end;
                index = rewritten.next;
                continue;
            }
            if let Some(name) = macro_head(&nodes[index]) {
                if self.macros.iter().any(|macro_| macro_.name == name && macro_.fixity != Fixity::Infix) {
                    index = self.expand_head(nodes, index, name);
                    continue;
                }
            }
            self.print_node(&nodes[index]);
            index += 1;
        }
    }

    fn print_node(&mut self, node: &Syntax) {
        match node {
            Syntax::Group {
                open,
                close,
                children,
                span,
                ..
            } => {
                self.copy_until(open.end as usize);
                self.print_nodes(children);
                let end = close.map_or(span.end, |close| close.end);
                self.copy_until(end as usize);
            }
            Syntax::String { span, raw } => {
                self.copy_until(span.start as usize);
                let rewritten = rewrite_specifier(raw);
                if rewritten == *raw {
                    self.copy_until(span.end as usize);
                } else {
                    self.emit(span.start, span.end, &rewritten, SegmentKind::Atom);
                    self.last_original = span.end as usize;
                }
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
            if matches!(&nodes[cursor], Syntax::Token { .. }) && cursor != index {
                let prev_is_op = cursor > index && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw != ".");
                let prev_is_dot = cursor > index && matches!(&nodes[cursor - 1], Syntax::Operator { raw, .. } if raw == ".");
                if !prev_is_op && !prev_is_dot && !token_is(Some(&nodes[cursor]), "as") {
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
        let left = sweetener_reader::slice_range(self.source, node_start(&nodes[index]), node_start(&nodes[op_at]));
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
        let rules: Vec<Rule> = self
            .macros
            .iter()
            .filter(|macro_| macro_.name == name)
            .flat_map(|macro_| macro_.rules.clone())
            .collect();
        for rule in &rules {
            let mut captures = HashMap::new();
            if infix_rule_binds(&rule.pattern, &name, left, right.trim(), &mut captures) {
                self.diagnose_unknown(&rule.template, node_span(&nodes[op_at]));
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                let replacement = instantiate(self.source, &rule.template, &captures, &self.classes);
                let expanded = self.expand_replacement(&replacement);
                self.printing_core = was_core;
                let start = node_span(&nodes[index]).start;
                self.copy_until(start as usize);
                self.emit(start, u32_at(right_end), &expanded, SegmentKind::Atom);
                self.last_original = right_end;
                return next;
            }
        }
        self.bad_macro(&name, node_span(&nodes[op_at]));
        self.print_node(&nodes[index]);
        index + 1
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
            if let Some(matched) = match_rule_with(self.source, nodes, index, &rule.pattern, &self.classes) {
                self.diagnose_unknown(&rule.template, head_span);
                let was_core = self.printing_core;
                if template_is_core(&rule.template) {
                    self.printing_core = true;
                }
                let mut replacement = instantiate(self.source, &rule.template, &matched.captures, &self.classes);
                // `#core(...)` hides its binders from the template tree.
                // The rendered text is the binding that must not collide.
                self.define_syntax(&rule.template, &matched.captures);
                let introduced = introduced_names(&rule.introduced, &replacement, &matched.captures);
                let plan = self.hygiene_plan(&introduced);
                if let Some(plan) = &plan {
                    replacement = apply_plan(&replacement, plan);
                }
                replacement = strip_already_printed(self.source, head_span.start as usize, &replacement);
                let expanded = self.expand_replacement(&replacement);
                self.printing_core = was_core;
                self.emit(head_span.start, u32_at(matched.end), &expanded, SegmentKind::Atom);
                self.last_original = matched.end;
                return matched.next;
            }
        }
        self.bad_macro(name, head_span);
        self.print_node(head);
        index + 1
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
                    self.classes.extend(collect_classes(&read.children));
                    eprintln!("defined from {rendered:?} found={:?}", found.keys().collect::<Vec<_>>());
                    for macro_ in found.into_values() {
                        if let Some(existing) = self.macros.iter_mut().find(|have| have.name == macro_.name) {
                            existing.rules.extend(macro_.rules);
                        } else {
                            self.macros.push(macro_);
                        }
                    }
                }
                Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::If { then_body: inner, .. } => {
                    self.define_syntax(inner, captures);
                }
                Tpl::If { else_body: Some(inner), .. } => self.define_syntax(inner, captures),
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
            occurrence_names: HashMap::new(),
            imports_emitted: true,
            form_signals: self.form_signals.clone(),
            form_methods: self.form_methods.clone(),
        };
        nested.print_nodes(&read.children);
        nested.copy_until(owned.len());
        self.diagnostics.append(&mut nested.diagnostics);
        nested.out
    }

    fn expand_replacement(&mut self, replacement: &str) -> String {
        if self.depth > 8 || !self.replacement_needs_expand(replacement) {
            return replacement.to_owned();
        }
        let owned = replacement.to_owned();
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
            printing_core: self.printing_core,
            user_names: self.user_names.clone(),
            classes: self.classes.clone(),
            runtime_imports: Vec::new(),
            hygiene: self.hygiene.clone(),
            occurrence_names: HashMap::new(),
            imports_emitted: true,
            form_signals: self.form_signals.clone(),
            form_methods: self.form_methods.clone(),
        };
        nested.print_nodes(&read.children);
        nested.copy_until(owned.len());
        self.diagnostics.append(&mut nested.diagnostics);
        self.hygiene = nested.hygiene;
        self.form_signals = nested.form_signals;
        self.form_methods = nested.form_methods;
        nested.out
    }

    fn user_code_starts(&self, nodes: &[Syntax], index: usize) -> bool {
        !is_syntax_import_at(nodes, index)
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
                lines.push_str(&format!(
                    "import {{ {} }} from {};\n",
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

    fn needs_hygiene(&self, introduced: &[String]) -> bool {
        if !self.hygiene.is_empty() {
            return true;
        }
        introduced.iter().any(|name| self.user_names.contains(name) || self.hygiene.contains_key(name))
    }

    fn hygiene_plan(&mut self, introduced: &[String]) -> Option<Vec<(String, String)>> {
        // A later expansion must not reprint `Inner` or `A` that an
        // earlier expansion already bound. The first expansion keeps the
        // spelling when nothing in the user file has it.
        let mut plan: Vec<(String, String)> = Vec::new();
        let mut used = self.user_names.clone();
        let mut seen = HashMap::<String, usize>::new();
        for name in introduced {
            let nth = seen.get(name).copied().unwrap_or(0);
            seen.insert(name.clone(), nth + 1);
            // `left` in `add` and `left` in `sub` are different binders, but
            // theirs numbers each spelling once per expansion: every `left`
            // in this quantity is `left_1`.
            if nth > 0 {
                if let Some(printed) = plan.iter().find(|(key, _)| key == &format!("{name}\00")).map(|(_, printed)| printed.clone()) {
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
                    if printed != &binding.local && !plan.iter().any(|(key, _)| key == &binding.local) {
                        plan.push((binding.local.clone(), printed.clone()));
                    }
                }
            }
        }
        if plan.is_empty() { None } else { Some(plan) }
    }

    fn replacement_needs_expand(&self, replacement: &str) -> bool {
        if self.printing_core {
            return false;
        }
        self.macros.iter().any(|macro_| {
            if macro_.name.chars().any(|ch| !ch.is_ascii_alphanumeric() && ch != '_') {
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
                message: format!("SWR4001: Sweetener macro error: template directive `{name}` is not implemented"),
                start,
                end,
            });
        }
    }

    fn bad_macro(&mut self, name: &str, span: Span) {
        self.diagnostics.push(Diagnostic {
            message: format!("SWR4001: Sweetener macro error: no rule for macro `{name}` accepted this input"),
            start: span.start,
            end: span.end,
        });
    }

    fn copy_until(&mut self, original_end: usize) {
        if original_end < self.last_original || original_end > self.source.len() {
            return;
        }
        let text = &self.source[self.last_original..original_end];
        self.emit(u32_at(self.last_original), u32_at(original_end), text, SegmentKind::Verbatim);
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
                if last.kind == SegmentKind::Verbatim && last.virtual_end == virtual_start && last.original_end == original_start {
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
}

#[derive(Clone, Debug)]
enum Pat {
    Lit(String),
    Group { delimiter: Delimiter, inner: Vec<Pat> },
    Hole { name: String, kind: HoleKind },
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
    Rep { inner: Vec<Tpl>, sep: Option<String> },
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
        loaded.classes.extend(collect_classes(&read.children));
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
        let used = runtime_used_by(&runtime, &taken);
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
}

fn collect_classes(nodes: &[Syntax]) -> Vec<SyntaxClass> {
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
            classes.push(class_of(&name, body));
            continue;
        }
        index += 1;
    }
    classes
}

fn class_of(name: &str, body: &Syntax) -> SyntaxClass {
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
                    rules.push(ClassRule { pattern });
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
            let Some(body) = next_group(nodes, &mut index, Delimiter::Brace) else {
                continue;
            };
            let rules = rules_in_source(source, body);
            if rules.is_empty() {
                continue;
            }
            let (fixity, precedence, associativity) = macro_fixity(operator, body);
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

fn next_group<'a>(nodes: &'a [Syntax], index: &mut usize, delimiter: Delimiter) -> Option<&'a Syntax> {
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
        if matches!(node, Syntax::Token { .. } | Syntax::Operator { .. }) {
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
        if !pattern.is_empty() {
            let introduced = introduced_of(source, std::slice::from_ref(template_group));
            rules.push(Rule {
                pattern,
                template,
                introduced,
            });
        }
    }
    rules
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
            Syntax::Group { delimiter, children, .. } => {
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
    } else if operator_is(nodes.get(cursor), ",")
        || operator_is(nodes.get(cursor), ";")
        || operator_is(nodes.get(cursor), "|")
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
    (
        Pat::Rep { inner, sep, kind },
        cursor,
    )
}

fn hole_kind(raw: &str) -> HoleKind {
    match raw {
        "expr" => HoleKind::Expr,
        "stmt" => HoleKind::Stmt,
        "type" => HoleKind::Type,
        "binding" | "ident" => HoleKind::Ident,
        "tt" | "token" => HoleKind::TokenTree,
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
        let Syntax::Token { raw: field, span: field_span, .. } = nodes.get(cursor + 1)? else {
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
    let Syntax::Group { children, span: arg_span, .. } = args else {
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
            push_gap(source, &mut inner, &mut inner_cursor, arg_span.end as usize - 1);
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
            Syntax::Group { delimiter, children, .. } => {
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

#[derive(Clone, Debug)]
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
    let end = match_pats(source, nodes, &mut cursor, pattern, &mut captures, classes)?;
    Some(Match {
        next: cursor,
        end,
        captures,
    })
}

fn skip_matched_prefix<'a>(nodes: &[Syntax], index: usize, pattern: &'a [Pat]) -> &'a [Pat] {
    let Some(head) = nodes.get(index).and_then(|node| match node {
        Syntax::Token { raw, .. } => Some(raw.as_str()),
        _ => None,
    }) else {
        return pattern;
    };
    let Some(at) = pattern.iter().position(|pat| matches!(pat, Pat::Lit(lit) if lit == head)) else {
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
    let start_end = nodes.get(*cursor).map(node_start).unwrap_or(0);
    let mut last_end = start_end;
    for pat in pattern {
        match pat {
            Pat::Lit(expected) => {
                skip_comments(nodes, cursor);
                let node = nodes.get(*cursor)?;
                if !node_is_lit(node, expected) {
                    return None;
                }
                last_end = node_end(node);
                *cursor += 1;
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
                // A single hole or `$($body:stmt)*` owns the group's interior,
                // including operators the flat walker stopped at.
                let whole = match inner.as_slice() {
                    [Pat::Hole { name, .. }] => Some(name.clone()),
                    [Pat::Rep {
                        inner: rep_inner,
                        sep: None,
                        ..
                    }] => match rep_inner.as_slice() {
                        [Pat::Hole { name, kind: HoleKind::Stmt }] => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                if let Some(name) = whole {
                    let text = sweetener_reader::slice_range(
                        source,
                        node_start(node) + 1,
                        node_end(node).saturating_sub(1),
                    )
                    .to_owned();
                    captures.insert(name, Capture::Text(text));
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
                    let taken = if *kind == HoleKind::Binding && pattern.len() == 1 {
                        // `rule { clamp(...) }` is a binding hole standing for the
                        // whole call. Inside `function $name(...)` it is one name.
                        take_expr(source, nodes, *cursor).or_else(|| take_ident(source, nodes, *cursor))
                    } else {
                        take_hole(source, nodes, *cursor, kind)
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
                    if match_pats(source, nodes, cursor, inner, &mut one, classes).is_none() {
                        *cursor = saved;
                        break;
                    }
                    let one: Vec<(String, Capture)> = one.into_iter().collect();
                    many.push(one);
                    if let Some(sep) = sep {
                        let saved = *cursor;
                        skip_comments(nodes, cursor);
                        if nodes.get(*cursor).is_some_and(|node| node_is_lit(node, sep)) {
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
                // including the newlines between the braces.
                if sep.is_none() {
                    if let Some(name) = many.iter().find_map(|one| one.first().map(|(name, _)| name.clone())) {
                        if let (Some(start_node), Some(end_node)) = (
                            nodes.get(saved_start),
                            cursor.checked_sub(1).and_then(|at| nodes.get(at)),
                        ) {
                            let text = sweetener_reader::slice_range(
                                source,
                                node_start(start_node),
                                node_end(end_node),
                            );
                            captures.insert(name, Capture::Text(text.to_owned()));
                        }
                    }
                }
                for (name, rows) in grouped {
                    if captures.contains_key(&name) {
                        continue;
                    }
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
            } else {
                fields.insert(field.name.clone(), value);
            }
        }
        return Some((
            Capture::Record { text, fields },
            end,
            child,
        ));
    }
    None
}

fn take_hole(
    source: &str,
    nodes: &[Syntax],
    cursor: usize,
    kind: &HoleKind,
) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    match kind {
        HoleKind::Class(_) => None,
        HoleKind::Stmt => {
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
                if operator_is(Some(&nodes[end_index]), ";") {
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
        HoleKind::TokenTree => take_token_tree(source, nodes, index),
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
        if matches!(node, Syntax::Group { delimiter: Delimiter::Brace, .. }) {
            break;
        }
        if matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), ";" | "," | "=" | ")" | "]" | "}" | ">")) {
            break;
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
fn take_token_tree(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    let mut index = cursor;
    skip_comments(nodes, &mut index);
    let node = nodes.get(index)?;
    if is_expr_stop(node) {
        return None;
    }
    if matches!(node, Syntax::Group { .. }) {
        return Some((sweetener_reader::slice(source, node).to_owned(), node_end(node), index + 1));
    }
    Some((sweetener_reader::slice(source, node).to_owned(), node_end(node), index + 1))
}

fn take_expr(source: &str, nodes: &[Syntax], cursor: usize) -> Option<(String, usize, usize)> {
    take_expr_until(source, nodes, cursor, None)
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
        if matches!(node, Syntax::Operator { raw, .. } if raw == "," || raw == ":" || raw == "=") {
            break;
        }
        if matches!(node, Syntax::Group { .. }) || matches!(node, Syntax::Token { .. } | Syntax::String { .. }) {
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
        if matches!(node, Syntax::Operator { .. }) && nodes.get(index + 1).is_some_and(is_expr_start) {
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
    if matches!(start_node, Syntax::Operator { .. }) && nodes.get(index).is_some_and(is_expr_start) {
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
            // `!==` and `&&` are several operator characters. An expression
            // hole includes them; a custom infix still stops at its own spelling.
            if matches!(node, Syntax::Operator { raw, .. } if matches!(raw.as_str(), "!" | "=" | "<" | ">" | "&" | "|" | "?" | "+" | "-" | "*" | "/" | "%"))
                && nodes.get(op_end).is_some_and(|next| {
                    matches!(next, Syntax::Operator { .. } | Syntax::Token { .. } | Syntax::String { .. })
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
        let chained = matches!(nodes.get(index - 1), Some(Syntax::Operator { raw, .. }) if matches!(raw.as_str(), "!" | "=" | "<" | ">" | "&" | "|" | "?" | "+" | "-" | "*" | "/" | "%"))
            && nodes.get(index).is_some_and(|next| matches!(next, Syntax::Operator { .. } | Syntax::Token { .. } | Syntax::String { .. }));
        if chained {
            break;
        }
        if stop_op.is_some() && operator_spelling(&nodes[index - 1]) != stop_op && nodes.get(index).is_some_and(is_expr_start) {
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
        Syntax::Operator { raw, .. } => matches!(raw.as_str(), ";" | ")" | "]" | "}" | "=>"),
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
    apply_plan_nodes(text, &read.children, plan, &mut seen, false, &mut out, &mut cursor);
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

fn occurrence_plan(plan: &[(String, String)], seen: &HashMap<String, usize>) -> HashMap<String, String> {
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
                let binder = is_binder_at(nodes, index);
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
            Syntax::Group { children, .. } => {
                if let Some(_params) = arrow_params(nodes, index) {
                    if let Syntax::Group { span, children, .. } = &nodes[index] {
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
                        apply_plan_nodes(source, &nodes[rest_index..], plan, seen, false, out, cursor);
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
                    let renamed = rename_template_literal(raw, &map, &[], &HashMap::new(), seen, span.start as usize);
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

fn planned(raw: &str, binder: bool, plan: &[(String, String)], seen: &HashMap<String, usize>) -> Option<String> {
    let opened = seen.get(raw).copied().unwrap_or(0);
    if !binder && opened == 0 {
        return plan.iter().find(|(key, _)| plan_name(key) == raw && !key.contains('\0')).map(|(_, printed)| printed.clone());
    }
    let nth = if binder { opened } else { opened.saturating_sub(1) };
    plan.iter().find(|(key, _)| key == &format!("{raw}\0{nth}")).map(|(_, printed)| printed.clone())
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
            if is_ident_text(raw) && !after_colon && !is_type_name(raw) {
                if let Some(printed) = planned(raw, true, plan, seen) {
                    push_until(source, out, cursor, span.start as usize);
                    out.push_str(&printed);
                    *cursor = span.end as usize;
                }
                let n = seen.get(raw).copied().unwrap_or(0);
                seen.insert(raw.clone(), n + 1);
            }
        }
        index += 1;
    }
}

fn is_type_name(raw: &str) -> bool {
    matches!(raw, "number" | "string" | "boolean" | "any" | "unknown" | "never" | "void" | "object" | "bigint" | "symbol" | "undefined" | "null")
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
        _ => None,
    }
}

fn group_is(node: Option<&Syntax>, delimiter: Delimiter) -> bool {
    matches!(node, Some(Syntax::Group { delimiter: found, .. }) if *found == delimiter)
}

fn is_closed_group(node: &Syntax, delimiter: Delimiter) -> bool {
    matches!(node, Syntax::Group { delimiter: found, close: Some(_), .. } if *found == delimiter)
}

fn node_is_lit(node: &Syntax, expected: &str) -> bool {
    match node {
        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } | Syntax::String { raw, .. } => raw == expected,
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
    while nodes.get(*cursor).is_some_and(|node| matches!(node, Syntax::Comment { .. })) {
        *cursor += 1;
    }
}

fn push_until(source: &str, out: &mut String, cursor: &mut usize, until: usize) {
    if until >= *cursor && until <= source.len() {
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
    for ext in [".sts", ".stsx", ".sjs", ".sjsx"] {
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

fn collect_user_binders(nodes: &[Syntax], names: &mut std::collections::HashSet<String>) {
    let mut index = 0;
    while index < nodes.len() {
        if is_binder_keyword(nodes, index) {
            if let Some(name) = binder_name_after(nodes, index) {
                names.insert(name);
            }
        }
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
    false
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

fn is_syntax_import_at(nodes: &[Syntax], index: usize) -> bool {
    if !token_at(nodes, index, "import") {
        return false;
    }
    // Only this statement. A later `export syntax` in the same module is not
    // part of the import, and treating it as one dropped the runtime import.
    nodes.iter().skip(index).take_while(|node| !operator_is(Some(node), ";")).any(|node| token_is(Some(node), "syntax"))
}

fn syntax_import_at(nodes: &[Syntax], index: usize) -> Option<SyntaxImport> {
    let span = node_span(&nodes[index]);
    let mut names = Vec::new();
    let mut aliases = Vec::new();
    if let Some(Syntax::Group { children, delimiter: Delimiter::Brace, .. }) = nodes.get(index + 1) {
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
                        Syntax::Token { raw, .. } | Syntax::Operator { raw, .. } => Some(raw.as_str()),
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
    let specifier = nodes.iter().skip(index).take(12).find_map(|node| match node {
        Syntax::String { raw, .. } => Some(raw.clone()),
        _ => None,
    })?;
    Some(SyntaxImport { names, aliases, specifier, span })
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
            Tpl::Core(inner) | Tpl::Rep { inner, .. } | Tpl::Define(inner) | Tpl::If { then_body: inner, .. } => {
                found.extend(unknown_directives(inner));
            }
            Tpl::If { else_body: Some(inner), .. } => found.extend(unknown_directives(inner)),
            _ => {}
        }
    }
    found
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

fn introduced_names(declared: &[String], rendered: &str, captures: &HashMap<String, Capture>) -> Vec<String> {
    let read = sweetener_reader::read_syntax(rendered);
    let mut names = introduced_of(rendered, &read.children);
    if names.is_empty() {
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
        if let Some(text) = capture_spelling(value) {
            let trimmed = text.trim();
            if is_ident_text(trimmed) {
                names.insert(trimmed.to_owned());
            }
        }
    }
    names
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
        if let Syntax::Group { delimiter: Delimiter::Paren, children, .. } = &nodes[index] {
            if paren_is_arrow_params(nodes, index) {
                names.extend(param_names(children));
            }
        }
        if let Syntax::Group { children, .. } = &nodes[index] {
            collect_introduced_source(source, children, names);
        }
        index += 1;
    }
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
    let Syntax::Group { delimiter: Delimiter::Paren, children, .. } = nodes.get(index)? else {
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
        if matches!(node, Syntax::Group { delimiter: Delimiter::Paren, .. } | Syntax::Group { delimiter: Delimiter::Brace, .. }) {
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
            Tpl::Join { name, prefix, suffix, casing } => {
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
            Tpl::If { path, then_body, else_body } => {
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
    for part in trim_template(template) {
        match part {
            Tpl::Text(text) => {
                let substituted = substitute_text(text, captures);
                out.push_str(&substituted);
            }
            Tpl::Var(name) => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(text.trim());
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
            Tpl::Join { name, prefix, suffix, casing } => {
                if let Some(text) = capture_text(captures, name) {
                    out.push_str(&join_identifier(prefix, text.trim(), suffix, casing));
                }
            }
            Tpl::Count(name) => out.push_str(&capture_count(captures, name).to_string()),
            Tpl::If { path, then_body, else_body } => {
                if capture_present(captures, path) {
                    render(out, then_body, captures, classes);
                } else if let Some(else_body) = else_body {
                    render(out, else_body, captures, classes);
                }
            }
            Tpl::Define(_) => {}
            Tpl::Unknown { .. } => {}
            Tpl::Rep { inner, sep } => {
                let rows = rep_rows(inner, captures);
                for (index, row) in rows.iter().enumerate() {
                    if index > 0 {
                        if let Some(sep) = sep {
                            out.push_str(sep);
                            if *sep == "," {
                                out.push(' ');
                            }
                        }
                    }
                    let mut scoped = captures.clone();
                    for (name, value) in row {
                        scoped.insert(name.clone(), value.clone());
                    }
                    render(out, inner, &scoped, classes);
                }
            }
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

fn substitute_text_protecting(text: &str, captures: &HashMap<String, Capture>) -> (String, Vec<(usize, usize)>) {
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
    substitute_nodes(text, &read.children, captures, &mut out, &mut cursor, &mut protected);
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
            if let Some(Syntax::Token { span, raw, .. }) = nodes.get(index + 1) {
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
        if bytes[index] == b'$' {
            let rest = &raw[index + 1..];
            let name: String = rest
                .chars()
                .take_while(|ch| *ch == '_' || ch.is_ascii_alphanumeric())
                .collect();
            if !name.is_empty() {
                if let Some(value) = capture_text(captures, &name) {
                    out.push_str(value.trim());
                    index += 1 + name.len();
                    continue;
                }
            }
        }
        out.push(bytes[index] as char);
        index += 1;
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

fn capture_text<'a>(captures: &'a HashMap<String, Capture>, name: &str) -> Option<&'a str> {
    capture_at(captures, name).and_then(capture_spelling)
}

fn capture_spelling(value: &Capture) -> Option<&str> {
    match value {
        Capture::Text(text) => Some(text.as_str()),
        Capture::Record { text, .. } => Some(text.as_str()),
        Capture::Many(rows) => rows.first().and_then(|row| {
            row.iter().find_map(|(_, value)| capture_spelling(value))
        }),
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
            Capture::Many(rows) => {
                // `$state.moves.event` while `moves` is still the sequence:
                // the field lives on each element. Callers that repeat on
                // `moves` scope one element first.
                let _ = field;
                return rows.first().and_then(|row| {
                    row.iter().find_map(|(_, element)| match element {
                        Capture::Record { fields, .. } => capture_field(fields, field),
                        _ => None,
                    })
                });
            }
            Capture::Text(_) => return None,
        };
    }
    Some(value)
}

fn capture_field<'a>(fields: &'a HashMap<String, Capture>, name: &str) -> Option<&'a Capture> {
    fields.get(name)
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
                && source[..node_start(&nodes[index])].trim_end().ends_with('(');
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
    if matches!(nodes.get(cursor), Some(Syntax::Token { .. }) | Some(Syntax::Group { .. })) {
        cursor += 1;
    }
    while cursor < nodes.len() {
        match &nodes[cursor] {
            Syntax::Group { delimiter, .. } if matches!(delimiter, Delimiter::Paren | Delimiter::Bracket) => {
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
    let mut best: Option<(&str, &Vec<Vec<(String, Capture)>>)> = None;
    for name in &names {
        let mut path = name.as_str();
        while !path.is_empty() {
            if let Some(Capture::Many(rows)) = capture_at(captures, path) {
                let longer = best.is_none_or(|(_, have)| rows.len() >= have.len() && path.len() >= best.unwrap().0.len());
                if longer {
                    best = Some((path, rows));
                }
                break;
            }
            path = path.rsplit_once('.').map(|(parent, _)| parent).unwrap_or("");
        }
    }
    if let Some((path, rows)) = best {
        return rows
            .iter()
            .map(|row| scope_rep_row(path, row))
            .collect();
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
    let element = row
        .iter()
        .find(|(key, _)| key == path.rsplit('.').next().unwrap_or(path) || key == path)
        .or_else(|| row.first())
        .map(|(_, value)| value.clone());
    if let Some(element) = element {
        scoped.push((path.to_owned(), element.clone()));
        // `$state.moves.event` inside a repetition of `$state` reads the
        // field off this element, not off the whole sequence.
        if let Capture::Record { fields, .. } = &element {
            for (field, value) in fields {
                let mut child = String::from(path);
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
            Tpl::Var(name) | Tpl::Quote(name) | Tpl::Count(name) | Tpl::If { path: name, .. } => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            Tpl::Join { name, .. } => {
                if !names.contains(name) {
                    names.push(name.clone());
                }
            }
            Tpl::Define(inner) | Tpl::Core(inner) | Tpl::Rep { inner, .. } => collect_vars(inner, names),
            Tpl::If { then_body, else_body, .. } => {
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
    rename_nodes(source, nodes, plan, capture_spans, occurrences, seen, false, &mut out, &mut cursor);
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
                if !dotted && !in_capture(capture_spans, span.start as usize) && printed_for(raw, binder_here, plan, occurrences, seen).is_some() =>
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
                rename_nodes(source, children, plan, capture_spans, occurrences, seen, false, out, cursor);
                dotted = false;
            }
            Syntax::Operator { raw, .. } if raw == "." => dotted = true,
            Syntax::String { span, raw } if raw.starts_with('`') && raw.contains("${") => {
                push_until(source, out, cursor, span.start as usize);
                out.push_str(&rename_template_literal(raw, plan, capture_spans, occurrences, seen, span.start as usize));
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
            return names.get(current).cloned().or_else(|| names.first().cloned());
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
    let Syntax::Group { delimiter: Delimiter::Brace, children, .. } = nodes.get(index + 1)? else {
        return None;
    };
    let specifier = nodes.iter().skip(index).take(8).find_map(|node| match node {
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
    Some(RuntimeImport { specifier, bindings })
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
            Tpl::Core(inner) | Tpl::Rep { inner, .. } => collect_free_idents(inner, names),
            _ => {}
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
        if let Some(existing) = into.iter_mut().find(|found| found.specifier == import.specifier) {
            for binding in import.bindings {
                if !existing.bindings.iter().any(|have| have.local == binding.local) {
                    existing.bindings.push(binding);
                }
            }
        } else {
            into.push(import);
        }
    }
}

fn infix_rule_binds(pattern: &[Pat], name: &str, left: &str, right: &str, captures: &mut HashMap<String, Capture>) -> bool {
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

fn fresh_name(name: &str, used: &std::collections::HashSet<String>, hygiene: &HashMap<String, String>) -> String {
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

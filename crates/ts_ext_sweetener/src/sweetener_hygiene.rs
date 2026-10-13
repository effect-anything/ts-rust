//! Output-based hygiene, upstream's `planHygienicRenames` on printed text.
//!
//! Expansion does not choose names. Every binder a template introduces is
//! printed as a placeholder, one class per (expansion, spelling), and every
//! use the template ties to that binder gets the same placeholder. Once the
//! whole file is printed, TypeScript parses it and one assignment runs:
//!
//! - candidates are the placeholder classes seen in a binder position, in
//!   first-occurrence order;
//! - a class bound by an `export` declaration or named by `export { .. }`
//!   is published and keeps its spelling;
//! - unavailable names are every other identifier the output spells in the
//!   same namespace (value or label), properties excluded;
//! - each class takes its spelling, or `name_N` with the first free `N`,
//!   never a name another class of that namespace already took;
//! - a shorthand `{ x }` prints `{ x: x_N }`, and `import { a }` prints
//!   `import { a as a_N }`; a property position keeps the spelling.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicUsize, Ordering};

use ts_goport::astdata::SyntaxKind;
use ts_goport::core::Node;
use ts_goport::flags::{ModifierFlags, ScriptKind};
use ts_goport::frontend::parser::{SourceFileParseOptions, parse_source_file};

use crate::Segment;

const PREFIX: &str = "__swh";
static NEXT: AtomicUsize = AtomicUsize::new(0);

/// A fresh placeholder for one introduced binder named `name`.
pub(crate) fn placeholder(name: &str) -> String {
    // Tests force an expander panic through a `#fresh` hint. Every fresh
    // binder, including one a later pass reprints, comes through here, and
    // the expander never produces this hint itself.
    #[cfg(test)]
    if name == crate::sweetener_expand::PANIC_FRESH_HINT {
        panic!("forced expander panic");
    }
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    format!("{PREFIX}{id}_{name}")
}

thread_local! {
    static SCOPE: std::cell::RefCell<Option<HashMap<String, String>>> =
        const { std::cell::RefCell::new(None) };
    static REMEMBERED: std::cell::RefCell<HashMap<String, String>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Run `f` as one expansion: `scoped` gives each spelling one placeholder.
pub(crate) fn in_scope<T>(f: impl FnOnce() -> T) -> T {
    let saved = SCOPE.with(|scope| scope.replace(Some(HashMap::new())));
    let out = f();
    SCOPE.with(|scope| *scope.borrow_mut() = saved);
    out
}

/// Hand `printed` to the form that will print `name`'s declaration.
pub(crate) fn remember(name: &str, printed: &str) {
    REMEMBERED.with(|map| {
        map.borrow_mut().insert(name.to_owned(), printed.to_owned());
    });
}

pub(crate) fn is_placeholder(text: &str) -> bool {
    spelling(text).is_some()
}

/// The spelling a placeholder stands for.
fn spelling(text: &str) -> Option<&str> {
    let rest = text.strip_prefix(PREFIX)?;
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    if digits == 0 {
        return None;
    }
    rest[digits..]
        .strip_prefix('_')
        .filter(|name| !name.is_empty())
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Space {
    Value,
    Label,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Binder,
    Property,
    Reference,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Occurrence {
    Identifier,
    Shorthand,
    Import,
}

struct Ident {
    text: String,
    start: usize,
    role: Role,
    space: Space,
    kind: Occurrence,
    published: bool,
    export_specifier: bool,
}

fn role_of(ident: Node, parent: Node) -> Role {
    let is_name = || parent.name() == ident;
    match parent.kind() {
        SyntaxKind::Parameter => {
            if is_name() {
                Role::Binder
            } else {
                Role::Property
            }
        }
        SyntaxKind::VariableDeclaration
        | SyntaxKind::FunctionDeclaration
        | SyntaxKind::FunctionExpression
        | SyntaxKind::ClassDeclaration
        | SyntaxKind::ClassExpression
        | SyntaxKind::InterfaceDeclaration
        | SyntaxKind::TypeAliasDeclaration
        | SyntaxKind::EnumDeclaration
        | SyntaxKind::ModuleDeclaration
        | SyntaxKind::TypeParameter
        | SyntaxKind::ImportClause
        | SyntaxKind::NamespaceImport
        | SyntaxKind::ImportEqualsDeclaration => {
            if is_name() {
                Role::Binder
            } else {
                Role::Reference
            }
        }
        SyntaxKind::BindingElement | SyntaxKind::ImportSpecifier => {
            if parent.property_name() == ident {
                Role::Property
            } else if is_name() {
                Role::Binder
            } else {
                Role::Reference
            }
        }
        SyntaxKind::ExportSpecifier
        | SyntaxKind::PropertyAssignment
        | SyntaxKind::MethodDeclaration
        | SyntaxKind::MethodSignature
        | SyntaxKind::PropertyDeclaration
        | SyntaxKind::PropertySignature
        | SyntaxKind::GetAccessor
        | SyntaxKind::SetAccessor
        | SyntaxKind::EnumMember
        | SyntaxKind::JsxAttribute
        | SyntaxKind::NamedTupleMember => {
            if is_name() {
                Role::Property
            } else {
                Role::Reference
            }
        }
        SyntaxKind::PropertyAccessExpression => {
            if is_name() {
                Role::Property
            } else {
                Role::Reference
            }
        }
        SyntaxKind::QualifiedName => {
            if first_child(parent) == Some(ident) {
                Role::Reference
            } else {
                Role::Property
            }
        }
        SyntaxKind::LabeledStatement => {
            if parent.label() == ident {
                Role::Binder
            } else {
                Role::Reference
            }
        }
        SyntaxKind::MetaProperty => Role::Property,
        _ => Role::Reference,
    }
}

fn first_child(node: Node) -> Option<Node> {
    let mut first = None;
    node.for_each_child(|child| {
        first = Some(child);
        true
    });
    first
}

fn space_of(ident: Node, parent: Node) -> Space {
    match parent.kind() {
        SyntaxKind::LabeledStatement if parent.label() == ident => Space::Label,
        SyntaxKind::BreakStatement | SyntaxKind::ContinueStatement => Space::Label,
        _ => Space::Value,
    }
}

/// Whether the declaration a binder belongs to carries `export`.
fn declares_public_name(parents: &[Node]) -> bool {
    for node in parents.iter().rev() {
        match node.kind() {
            SyntaxKind::VariableDeclaration
            | SyntaxKind::VariableDeclarationList
            | SyntaxKind::BindingElement
            | SyntaxKind::ObjectBindingPattern
            | SyntaxKind::ArrayBindingPattern => continue,
            _ => return node.modifier_flags().intersects(ModifierFlags::EXPORT),
        }
    }
    false
}

fn visit(node: Node, parents: &mut Vec<Node>, out: &mut Vec<Ident>) {
    if node.kind() == SyntaxKind::Identifier {
        if let Some(&parent) = parents.last() {
            let text = node.text().to_owned();
            let start = (node.end() as usize).saturating_sub(text.len());
            let role = role_of(node, parent);
            let kind =
                match parent.kind() {
                    SyntaxKind::ShorthandPropertyAssignment if parent.name() == node => {
                        Occurrence::Shorthand
                    }
                    SyntaxKind::BindingElement
                        if parent.name() == node
                            && parent.property_name().is_nil()
                            && parents.get(parents.len().wrapping_sub(2)).is_some_and(
                                |grand| grand.kind() == SyntaxKind::ObjectBindingPattern,
                            ) =>
                    {
                        Occurrence::Shorthand
                    }
                    SyntaxKind::ImportSpecifier if parent.property_name().is_nil() => {
                        Occurrence::Import
                    }
                    _ => Occurrence::Identifier,
                };
            out.push(Ident {
                text,
                start,
                role,
                space: space_of(node, parent),
                kind,
                published: role == Role::Binder && declares_public_name(parents),
                export_specifier: parent.kind() == SyntaxKind::ExportSpecifier,
            });
        }
        return;
    }
    parents.push(node);
    node.for_each_child(|child| {
        if child != node {
            visit(child, parents, out);
        }
        false
    });
    parents.pop();
}

fn identifiers(code: &str, tsx: bool) -> Vec<Ident> {
    let leaked: &'static str = Box::leak(code.to_owned().into_boxed_str());
    let opts = SourceFileParseOptions {
        file_name: if tsx { "/hygiene.tsx" } else { "/hygiene.ts" }.to_owned(),
        ..SourceFileParseOptions::default()
    };
    let kind = if tsx { ScriptKind::TSX } else { ScriptKind::TS };
    let file = parse_source_file(&opts, leaked, kind);
    let mut out = Vec::new();
    let mut parents = Vec::new();
    visit(file.root, &mut parents, &mut out);
    out.sort_by_key(|ident| ident.start);
    out
}

/// Assign every placeholder class its printed name and rewrite `code`.
/// Segment offsets move with the text.
pub(crate) fn assign(code: &str, segments: &mut [Segment], tsx: bool) -> String {
    if !code.contains(PREFIX) {
        return code.to_owned();
    }
    let idents = identifiers(code, tsx);

    let mut order: Vec<String> = Vec::new();
    let mut space: HashMap<String, Space> = HashMap::new();
    let mut binders: HashSet<String> = HashSet::new();
    let mut pinned: HashSet<String> = HashSet::new();
    let mut unavailable: HashMap<Space, HashSet<String>> = HashMap::new();
    let mut kinds: HashMap<usize, (Occurrence, Role)> = HashMap::new();
    for ident in &idents {
        kinds.insert(ident.start, (ident.kind, ident.role));
        if is_placeholder(&ident.text) {
            if ident.export_specifier {
                pinned.insert(ident.text.clone());
            }
            if ident.role == Role::Property {
                continue;
            }
            if !space.contains_key(&ident.text) {
                order.push(ident.text.clone());
                space.insert(ident.text.clone(), ident.space);
            }
            if ident.role == Role::Binder {
                binders.insert(ident.text.clone());
                if ident.published {
                    pinned.insert(ident.text.clone());
                }
            }
            continue;
        }
        if ident.role == Role::Property {
            continue;
        }
        unavailable
            .entry(ident.space)
            .or_default()
            .insert(ident.text.clone());
    }
    // A class never seen binding, or published, holds its spelling and
    // blocks it for the others.
    for key in &order {
        if pinned.contains(key) || !binders.contains(key) {
            let name = spelling(key).unwrap_or(key).to_owned();
            unavailable.entry(space[key]).or_default().insert(name);
        }
    }
    let mut names: HashMap<String, String> = HashMap::new();
    let mut taken: HashMap<Space, HashSet<String>> = HashMap::new();
    for key in &order {
        let base = spelling(key).unwrap_or(key).to_owned();
        if pinned.contains(key) || !binders.contains(key) {
            names.insert(key.clone(), base);
            continue;
        }
        let here = space[key];
        let blocked = unavailable.get(&here);
        let claimed = taken.entry(here).or_default();
        let mut candidate = base.clone();
        let mut suffix = 1;
        while blocked.is_some_and(|set| set.contains(&candidate)) || claimed.contains(&candidate) {
            candidate = format!("{base}_{suffix}");
            suffix += 1;
        }
        claimed.insert(candidate.clone());
        names.insert(key.clone(), candidate);
    }

    // Rewrite, tracking how far each old offset moved.
    let mut out = String::with_capacity(code.len());
    let mut edits: Vec<(usize, usize, usize)> = Vec::new();
    let mut cursor = 0;
    let mut position = 0;
    while let Some(found) = code[position..].find(PREFIX) {
        let at = position + found;
        let end = ident_end(code, at);
        if code[..at].ends_with("/*") && code[at..].starts_with(&format!("{PREFIX},")) {
            if let Some(close) = code[at..].find("*/") {
                let close = at + close;
                let renamed = code[at + PREFIX.len() + 1..close]
                    .split(',')
                    .filter(|key| !key.is_empty())
                    .any(|key| {
                        names
                            .get(key)
                            .is_some_and(|printed| Some(printed.as_str()) != spelling(key))
                    });
                out.push_str(&code[cursor..at - 2]);
                if !renamed {
                    out.push(',');
                }
                edits.push((at - 2, close + 2, out.len()));
                cursor = close + 2;
                position = cursor;
                continue;
            }
        }
        let boundary = !code[..at].chars().next_back().is_some_and(ident_char);
        let text = &code[at..end];
        if let (true, Some(name)) = (boundary, spelling(text)) {
            let (kind, role) = kinds
                .get(&at)
                .copied()
                .unwrap_or((Occurrence::Identifier, Role::Reference));
            let printed = if role == Role::Property {
                name.to_owned()
            } else {
                names.get(text).cloned().unwrap_or_else(|| name.to_owned())
            };
            let replacement = match kind {
                Occurrence::Shorthand if printed != name => format!("{name}: {printed}"),
                Occurrence::Import if printed != name => format!("{name} as {printed}"),
                _ => printed,
            };
            out.push_str(&code[cursor..at]);
            out.push_str(&replacement);
            edits.push((at, end, out.len()));
            cursor = end;
        }
        position = end.max(at + 1);
    }
    out.push_str(&code[cursor..]);
    let map = |offset: usize| -> usize {
        let mut moved = offset as isize;
        for (start, end, new_end) in &edits {
            if *end <= offset {
                moved = offset as isize + (*new_end as isize - *end as isize);
            } else if *start < offset {
                moved = *new_end as isize;
            } else {
                break;
            }
        }
        moved.max(0) as usize
    };
    for segment in segments.iter_mut() {
        segment.virtual_start = map(segment.virtual_start as usize) as u32;
        segment.virtual_end = map(segment.virtual_end as usize) as u32;
        // A rewrite inside a verbatim range changed its length: the bytes
        // no longer match one for one, so the range maps as a whole.
        if segment.kind == crate::SegmentKind::Verbatim
            && segment.virtual_end - segment.virtual_start
                != segment.original_end - segment.original_start
        {
            segment.kind = crate::SegmentKind::Atom;
        }
    }
    out
}

fn ident_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_alphanumeric()
}

fn ident_end(code: &str, at: usize) -> usize {
    code[at..]
        .char_indices()
        .find(|(_, ch)| !ident_char(*ch))
        .map_or(code.len(), |(offset, _)| at + offset)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(code: &str) -> String {
        assign(code, &mut [], true)
    }

    #[test]
    fn first_occurrence_keeps_the_spelling() {
        let a = placeholder("tmp");
        let b = placeholder("tmp");
        let out = run(&format!("const {a} = 1; const {b} = {a};"));
        assert_eq!(out, "const tmp = 1; const tmp_1 = tmp;");
    }

    #[test]
    fn user_names_block_and_properties_do_not() {
        let a = placeholder("id");
        let b = placeholder("x");
        let out = run(&format!(
            "const id = 0; obj.x; const {a} = 1; let {b} = {{ x: 1 }};"
        ));
        assert_eq!(
            out,
            "const id = 0; obj.x; const id_1 = 1; let x = { x: 1 };"
        );
    }

    #[test]
    fn exported_binders_keep_their_name() {
        let a = placeholder("value");
        let out = run(&format!("const value = 1; export const {a} = 2;"));
        assert_eq!(out, "const value = 1; export const value = 2;");
        let b = placeholder("v");
        let out = run(&format!("let v; function {b}() {{}} export {{ {b} }};"));
        assert_eq!(out, "let v; function v() {} export { v };");
    }

    #[test]
    fn labels_are_their_own_namespace() {
        let a = placeholder("outer");
        let out = run(&format!("const outer = 1; {a}: for (;;) {{ break {a}; }}"));
        assert_eq!(out, "const outer = 1; outer: for (;;) { break outer; }");
        let b = placeholder("outer");
        let out = run(&format!(
            "outer: while (1) {{ {b}: for (;;) {{ continue {b}; }} }}"
        ));
        assert_eq!(
            out,
            "outer: while (1) { outer_1: for (;;) { continue outer_1; } }"
        );
    }

    #[test]
    fn shorthand_and_import_bindings_keep_the_property() {
        let a = placeholder("x");
        let out = run(&format!("let x; const {{ {a} }} = o; use({a});"));
        assert_eq!(out, "let x; const { x: x_1 } = o; use(x_1);");
        let b = placeholder("a");
        let out = run(&format!("import {{ {b} }} from \"m\"; const a = {b};"));
        assert_eq!(out, "import { a as a_1 } from \"m\"; const a = a_1;");
    }

    #[test]
    fn jsx_text_and_attributes_are_not_names() {
        let a = placeholder("cache");
        let b = placeholder("className");
        let out = run(&format!(
            "const {a} = 1; let {b}; <div className={{{b}}}>nine cache slots</div>;"
        ));
        assert_eq!(
            out,
            "const cache = 1; let className; <div className={className}>nine cache slots</div>;"
        );
    }

    #[test]
    fn template_interpolations_count() {
        let a = placeholder("n");
        let out = run(&format!("const {a} = 1; `${{n}} ${{{a}}}`;"));
        assert_eq!(out, "const n_1 = 1; `${n} ${n_1}`;");
    }

    #[test]
    fn segments_move_with_the_text() {
        let a = placeholder("t");
        let code = format!("let {a} = 1; z;");
        let z = code.find('z').expect("z") as u32;
        let mut segments = [Segment {
            virtual_start: z,
            virtual_end: z + 1,
            original_start: 0,
            original_end: 1,
            kind: crate::SegmentKind::Verbatim,
        }];
        let out = assign(&code, &mut segments, false);
        assert_eq!(
            &out[segments[0].virtual_start as usize..segments[0].virtual_end as usize],
            "z"
        );
    }
}

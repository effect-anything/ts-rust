//! Every Sweetener diagnostic code: what it means, a minimal file that
//! triggers it, and how to fix it.
//!
//! This table is the one source. `tsc-ext explain <code>` prints an entry,
//! `ext/docs/diagnostics.md` is generated from it ([`markdown`]), and
//! `tests/catalog.rs` checks that every code the expander can emit is here,
//! that each `bad` example really reports its code, and that the generated
//! page is current (`TSC_EXT_UPDATE=1` rewrites it).

use std::fmt::Write as _;
/// How a code is reported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Error,
    Warning,
}

impl Level {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Level::Error => "error",
            Level::Warning => "warning",
        }
    }
}

/// One diagnostic code.
#[derive(Clone, Copy, Debug)]
pub struct Entry {
    /// `SWR1001`.
    pub code: &'static str,
    pub level: Level,
    /// One line, sentence case, no period.
    pub title: &'static str,
    /// What went wrong and why the expander refuses it.
    pub explanation: &'static str,
    /// A complete `.sts` file that reports `code`. Empty when no file can
    /// trigger it on purpose (an internal error).
    pub bad: &'static str,
    /// Other files next to `bad` (`(name, text)`), for import errors.
    pub files: &'static [(&'static str, &'static str)],
    /// How to fix it, ideally as the corrected file.
    pub fix: &'static str,
}

/// The entry for `code` (`SWR4001`, `swr4001` or `4001`).
#[must_use]
pub fn lookup(code: &str) -> Option<&'static Entry> {
    let code = code.trim().to_ascii_uppercase();
    let code = if code.starts_with("SWR") {
        code
    } else {
        format!("SWR{code}")
    };
    CATALOG.iter().find(|entry| entry.code == code)
}

/// The catalog as one Markdown page (`ext/docs/diagnostics.md`).
#[must_use]
pub fn markdown() -> String {
    let mut out = String::from(
        "# Sweetener diagnostics\n\n\
         <!-- Generated from crates/ts_ext_sweetener/src/catalog.rs by \
         `TSC_EXT_UPDATE=1 cargo test -p ts_ext_sweetener --test catalog`. Do not edit. -->\n\n\
         Every code the Sweetener expander reports. `tsc-ext explain <code>` prints one entry.\n\
         Diagnostics read `file:line:col: error SWRnnnn: message`. Codes 1xxx come from the reader \
         (delimiters and literals), 2xxx from a macro definition's patterns and templates, 3xxx \
         from binding contracts, 4xxx from expanding a use, and 5xxx from `for syntax` imports.\n\n",
    );
    for entry in CATALOG {
        let _ = writeln!(
            out,
            "- [{}](#{}) {}",
            entry.code,
            entry.code.to_ascii_lowercase(),
            entry.title
        );
    }
    for entry in CATALOG {
        let _ = write!(
            out,
            "\n## {}\n\n**{}** ({})\n\n{}\n",
            entry.code,
            entry.title,
            entry.level.as_str(),
            entry.explanation
        );
        if !entry.bad.is_empty() {
            for (name, text) in entry.files {
                let _ = write!(out, "\n`{name}`:\n\n```ts\n{text}```\n");
            }
            let _ = write!(out, "\nTriggers it:\n\n```ts\n{}```\n", entry.bad);
        }
        let _ = write!(out, "\nFix:\n\n{}\n", entry.fix);
    }
    out
}

const TWICE: &str = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n";

/// The codes, in numeric order.
pub const CATALOG: &[Entry] = &[
    Entry {
        code: "SWR1001",
        level: Level::Error,
        title: "Unterminated string, template or regular-expression literal",
        explanation: "The reader reached the end of the line (or file) inside a literal. Sweetener reads the whole file as tokens before any macro runs, so an open quote swallows the rest of the file and no macro after it can be found.",
        bad: "const s = \"open;\nconst x = 1;\n",
        files: &[],
        fix: "Close the literal: `const s = \"open\";`.",
    },
    Entry {
        code: "SWR1002",
        level: Level::Error,
        title: "Unexpected closing delimiter",
        explanation: "A `)`, `]` or `}` has no matching opener. Sweetener groups tokens by delimiters before it expands anything, so this is reported by the reader at that byte, not as a TypeScript parse error.",
        bad: "const a = f(1));\n",
        files: &[],
        fix: "Remove the extra closer, or add the opener it belongs to: `const a = f(1);`.",
    },
    Entry {
        code: "SWR1003",
        level: Level::Error,
        title: "Missing closing delimiter",
        explanation: "A `(`, `[` or `{` is never closed before the end of the file. The reader reports it at the opener.",
        bad: "function f() {\n  return 1;\n",
        files: &[],
        fix: "Close the group: add the missing `}` after `return 1;`.",
    },
    Entry {
        code: "SWR2001",
        level: Level::Error,
        title: "Malformed macro definition",
        explanation: "A `syntax` or `operator` definition is missing a part the reader needs: the name and category (`syntax name:category`), the body `{ ... }`, or a rule's pattern group `rule { ... }`.",
        bad: "export syntax twice {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n",
        files: &[],
        fix: "Write the category after the name: `export syntax twice:expr { ... }`. Categories are `expr`, `stmt` and `item`.",
    },
    Entry {
        code: "SWR2002",
        level: Level::Error,
        title: "Malformed pattern",
        explanation: "A rule pattern uses `$` without a capture (`$name:class`) or a repetition `$( ... )` without `*`, `+` or `?` after it.",
        bad: "export syntax foo:expr {\n  rule { foo($) } => { 1 }\n}\n",
        files: &[],
        fix: "Name the capture and its class: `rule { foo($x:expr) } => { $x }`, or close a repetition with `*`, `+` or `?`.",
    },
    Entry {
        code: "SWR2003",
        level: Level::Error,
        title: "Unknown syntax category",
        explanation: "The category after the macro name is not one Sweetener knows. It decides where the macro may be written and what its expansion must be.",
        bad: "export syntax foo:banana {\n  rule { foo() } => { 1 }\n}\n",
        files: &[],
        fix: "Use `expr` (an expression), `stmt` (a statement) or `item` (a module-level declaration): `export syntax foo:expr { ... }`.",
    },
    Entry {
        code: "SWR2004",
        level: Level::Error,
        title: "Capture bound more than once",
        explanation: "Two captures in one pattern have the same name, so the template could not tell which one `$x` means.",
        bad: "export syntax foo:expr {\n  rule { foo($x:expr, $x:expr) } => { $x }\n}\n",
        files: &[],
        fix: "Give each capture its own name: `rule { foo($a:expr, $b:expr) } => { $a + $b }`.",
    },
    Entry {
        code: "SWR2005",
        level: Level::Error,
        title: "Alternatives bind different captures",
        explanation: "The branches of a `|` alternative must bind the same captures with the same shape; otherwise a capture the template uses would be missing when the other branch matched.",
        bad: "export syntax foo:expr {\n  rule { foo(a $x:expr | b) } => { 1 }\n}\n",
        files: &[],
        fix: "Bind the same captures in every branch, or split the alternatives into separate rules: `rule { foo(a $x:expr) } => { $x }` and `rule { foo(b) } => { 1 }`.",
    },
    Entry {
        code: "SWR2006",
        level: Level::Error,
        title: "Repetition can match without consuming input",
        explanation: "A `*` or `+` repetition whose body can match nothing would loop forever, so the definition is refused.",
        bad: "export syntax foo:expr {\n  rule { foo($($($x:expr)?)*) } => { 1 }\n}\n",
        files: &[],
        fix: "Make every repetition consume at least one token: `rule { foo($($x:expr),*) } => { [$($x),*] }`.",
    },
    Entry {
        code: "SWR2008",
        level: Level::Error,
        title: "Unresolved syntax class",
        explanation: "A capture names a class (`$x:class`) that is neither built in (`expr`, `stmt`, `ident`, `binding`, `type`, `tt`, ...) nor a `syntax class` defined in or imported into this file.",
        bad: "export syntax foo:expr {\n  rule { foo($x:nosuch) } => { $x }\n}\n",
        files: &[],
        fix: "Use a built-in class (`$x:expr`), or define `syntax class nosuch { ... }` above the macro.",
    },
    Entry {
        code: "SWR2009",
        level: Level::Error,
        title: "Left-recursive syntax class",
        explanation: "A `syntax class` refers to itself before consuming any token, so matching it would recurse without end.",
        bad: "syntax class loop {\n  fields { v: tt; }\n  rule { $l:loop + $v:tt }\n}\n",
        files: &[],
        fix: "Consume a token before the recursive reference (`rule { ( $l:loop ) }`), or use a repetition: `rule { $($v:tt)+ }`.",
    },
    Entry {
        code: "SWR2010",
        level: Level::Error,
        title: "Invalid refinement",
        explanation: "A `refine` clause names a capture the rule does not bind, or uses a predicate Sweetener does not know.",
        bad: "export syntax foo:expr {\n  rule { foo($x:expr) }\n  refine $y length equal 1;\n  => { $x }\n}\n",
        files: &[],
        fix: "Refine a capture of the same rule: `refine $x length equal 1;`.",
    },
    Entry {
        code: "SWR2011",
        level: Level::Error,
        title: "Template refers to an unknown capture",
        explanation: "The template uses `$name`, but the rule's pattern binds no capture of that name.",
        bad: "export syntax foo:expr {\n  rule { foo($x:expr) } => { $y }\n}\nconst a = foo(1);\n",
        files: &[],
        fix: "Use a capture the pattern binds: `=> { $x }`.",
    },
    Entry {
        code: "SWR2012",
        level: Level::Error,
        title: "Capture has no such field",
        explanation: "`$capture.field` names a field that the capture's syntax class does not declare in its `fields { ... }`.",
        bad: "syntax class Pair {\n  fields { left: expr; right: expr; }\n  rule { [$left:expr, $right:expr] }\n}\nexport syntax foo:expr {\n  rule { foo($p:Pair) } => { $p.nope }\n}\n",
        files: &[],
        fix: "Use a declared field: `=> { $p.left }`.",
    },
    Entry {
        code: "SWR2013",
        level: Level::Error,
        title: "Capture used at the wrong repetition depth",
        explanation: "A capture bound inside `$( ... )*` holds a list. The template must use it inside a repetition of the same depth.",
        bad: "export syntax foo:expr {\n  rule { foo($($x:expr),*) } => { [$x] }\n}\n",
        files: &[],
        fix: "Repeat it as the pattern does: `=> { [$($x),*] }`.",
    },
    Entry {
        code: "SWR2014",
        level: Level::Error,
        title: "Template repetition has no driving capture",
        explanation: "A `$( ... )*` in a template repeats once per element of the repeated captures it contains. This one contains none, so its length is unknown.",
        bad: "export syntax foo:expr {\n  rule { foo($x:expr) } => { [$(1),*] }\n}\n",
        files: &[],
        fix: "Repeat over a repeated capture (`rule { foo($($x:expr),*) } => { [$($x),*] }`), or drop the repetition.",
    },
    Entry {
        code: "SWR2015",
        level: Level::Error,
        title: "Template repetition has incompatible driving captures",
        explanation: "One template repetition uses captures from different pattern repetitions, which can have different lengths.",
        bad: "export syntax foo:expr {\n  rule { foo($($x:expr),* ; $($y:expr),*) } => { [$($x + $y),*] }\n}\n",
        files: &[],
        fix: "Repeat each list on its own (`=> { [$($x),*, $($y),*] }`), or capture pairs in one repetition: `rule { foo($($x:expr : $y:expr),*) }`.",
    },
    Entry {
        code: "SWR2016",
        level: Level::Error,
        title: "Malformed template",
        explanation: "A template repetition `$( ... )` has no `*` or `+` after it, or an `#if` is not exactly `#if(present $capture)`.",
        bad: "export syntax twice:expr {\n  rule { twice($($x:expr),*) } => { [$($x),] }\n}\n",
        files: &[],
        fix: "Close the repetition with its separator and `*`: `=> { [$($x),*] }`. An optional capture is tested with `#if(present $x) { ... } #else { ... }`.",
    },
    Entry {
        code: "SWR2017",
        level: Level::Error,
        title: "Invalid argument for a template operation",
        explanation: "A template operation such as `#fresh(\"name\")`, `#capture($x)`, `#callsite($x)` or `#definition($x)` got an argument of the wrong kind: `#fresh` takes one identifier-like string, the scope operations take a capture.",
        bad: "export syntax w:stmt {\n  rule { w $a:ident; } => { #capture(result) = 1; }\n}\nw x;\n",
        files: &[],
        fix: "Pass a capture: `=> { #capture($a) = 1; }`, and give `#fresh` one word: `#fresh(\"tmp\")`.",
    },
    Entry {
        code: "SWR2018",
        level: Level::Error,
        title: "Unknown template operation",
        explanation: "The template calls `#name(...)`, but there is no such operation. Known ones include `#if`, `#else`, `#let`, `#parameterize`, `#fold`, `#fresh`, `#join`, `#count`, `#trim`, `#core`, `#capture`, `#callsite` and `#definition`.",
        bad: "export syntax foo:expr {\n  rule { foo($x:expr) } => { #nosuch($x) }\n}\nconst a = foo(1);\n",
        files: &[],
        fix: "Use a known operation, or write the code directly: `=> { $x }`.",
    },
    Entry {
        code: "SWR2019",
        level: Level::Error,
        title: "Malformed `for syntax` import",
        explanation: "A compile-time import must read `import { names } from \"module\" for syntax;`, optionally followed by `shadows core`. Anything else after `for syntax` is refused.",
        bad: "import { foo } from \"./x.sts\" for syntax as;\nconst a = 1;\n",
        files: &[],
        fix: "`import { foo } from \"./x.sts\" for syntax;`",
    },
    Entry {
        code: "SWR3002",
        level: Level::Error,
        title: "Malformed binding contract",
        explanation: "A `bind $name in <scope> as <kind> <space>;` clause names a capture the rule does not bind, or is missing a part (the scope, `as`, the kind or the space).",
        bad: "export syntax a:expr {\n  rule { a $x:expr }\n  bind $y in following as lexical value;\n  => { $x }\n}\n",
        files: &[],
        fix: "Bind a capture of this rule and write every part: `bind $name in following as lexical value;`.",
    },
    Entry {
        code: "SWR3003",
        level: Level::Error,
        title: "Invalid binding-contract binder",
        explanation: "The binder of a `bind` clause must be an `ident`, `binding` or class capture. An expression capture does not introduce a name.",
        bad: "export syntax c:stmt {\n  rule { c $x:expr $v:expr }\n  bind $x in $v as lexical value;\n  => { $v; }\n}\n",
        files: &[],
        fix: "Capture the name as a binding: `rule { c $x:binding $v:expr }`.",
    },
    Entry {
        code: "SWR3004",
        level: Level::Error,
        title: "Binding-contract repetitions are not alignable",
        explanation: "A `sequential` contract pairs each binder with the scope that follows it, so the binder and scope captures must repeat together.",
        bad: "export syntax foo:stmt {\n  rule { foo $a:binding in $b:expr ; }\n  bind $a in $b as sequential value;\n  => { 1; }\n}\n",
        files: &[],
        fix: "Use `lexical` for a single binder (`bind $a in $b as lexical value;`), or repeat binder and scope in one repetition.",
    },
    Entry {
        code: "SWR3005",
        level: Level::Error,
        title: "Binding space not allowed for this macro category",
        explanation: "An expression macro can bind values only. Types (and other spaces) can be bound by statement and item macros.",
        bad: "export syntax b:expr {\n  rule { b $x:ident $v:expr }\n  bind $x in $v as lexical type;\n  => { $v }\n}\n",
        files: &[],
        fix: "Bind a value (`as lexical value`), or make the macro a `stmt`/`item` macro.",
    },
    Entry {
        code: "SWR4001",
        level: Level::Error,
        title: "No rule of the macro accepted this use",
        explanation: "The macro is known here, but none of its rules matched the tokens at this use. The message says what the closest rule expected (its `expect` text, or the context it needs); a related location points where that rule stopped. `tsc-ext expand --trace` shows every rule tried and why it failed.",
        bad: "export syntax unless:stmt {\n  rule { unless ($c:expr) { $($b:stmt)* } } => { if (!($c)) { $($b)* } }\n}\nunless x { f(); }\n",
        files: &[],
        fix: "Write the use the way a rule expects (`unless (x) { f(); }`), or add a rule for this shape.",
    },
    Entry {
        code: "SWR4002",
        level: Level::Error,
        title: "`shadows core` on something that is not a core form",
        explanation: "`shadows core` lets a macro replace a built-in TypeScript form such as `function`. The macro's name is not one of those forms, so there is nothing to shadow.",
        bad: "export syntax hello:expr shadows core {\n  rule { hello } => { 1 }\n}\nconst x = hello;\n",
        files: &[],
        fix: "Drop `shadows core` (`export syntax hello:expr { ... }`); it is only for core forms.",
    },
    Entry {
        code: "SWR4003",
        level: Level::Error,
        title: "Import asks to shadow a core form the definition does not allow",
        explanation: "`import { name } ... for syntax shadows core` requests that the imported macro replace a core form, but its definition was not written with `shadows core`.",
        bad: "import { function } from \"./fn.sts\" for syntax shadows core;\nconst f = function (x: number) { x };\n",
        files: &[(
            "fn.sts",
            "export syntax function:expr {\n  rule { function($x:ident: $t:type) { $e:expr } } => { (($x: $t) => $e) }\n}\n",
        )],
        fix: "Write `shadows core` on the definition (`export syntax function:expr shadows core { ... }`), or import without it.",
    },
    Entry {
        code: "SWR4006",
        level: Level::Error,
        title: "Invalid operator definition",
        explanation: "An `operator` needs a `fixity` (`prefix`, `infix` or `postfix`), and an infix operator needs `associativity` (`left`, `right` or `none`) and a `precedence`. The message names the missing or invalid setting.",
        bad: "export operator (+++):expr {\n  fixity infix;\n  precedence 5;\n  rule { $a:expr +++ $b:expr } => { $a }\n}\n",
        files: &[],
        fix: "Add the missing setting: `associativity left;`.",
    },
    Entry {
        code: "SWR4008",
        level: Level::Error,
        title: "Unknown macro context",
        explanation: "A rule's `context` clause names a context Sweetener does not track. Known contexts are `async` and `generator`.",
        bad: "export syntax odd:expr {\n  rule { odd() }\n  context weird;\n  => { 1 }\n}\n",
        files: &[],
        fix: "`context async;` or `context generator;`, or remove the clause.",
    },
    Entry {
        code: "SWR4009",
        level: Level::Error,
        title: "Binding literal cannot resolve its path",
        explanation: "`literal path as Name;` must name a real global path (such as `globalThis.NaN`) for the spelling it rewrites.",
        bad: "export syntax foo:expr {\n  literal nosuch.thing as NaN;\n  rule { foo() } => { NaN }\n}\n",
        files: &[],
        fix: "`literal globalThis.NaN as NaN;`",
    },
    Entry {
        code: "SWR4010",
        level: Level::Error,
        title: "Macro defined twice in one module",
        explanation: "Two definitions in one file have the same name. Rules for one macro belong in one definition.",
        bad: "syntax foo:expr {\n  rule { foo() } => { 1 }\n}\nsyntax foo:expr {\n  rule { foo() } => { 2 }\n}\n",
        files: &[],
        fix: "Merge the rules into one definition, or rename one macro.",
    },
    Entry {
        code: "SWR4011",
        level: Level::Error,
        title: "Expression macro expanded to something that is not one expression",
        explanation: "A macro declared `expr` produced a statement (or several pieces). Its uses stand where an expression is read, so the expansion would not parse there.",
        bad: "export syntax foo:expr {\n  rule { foo() } => { if (x) {} }\n}\nconst a = foo();\n",
        files: &[],
        fix: "Make the template one expression (`=> { x ? 1 : 0 }`), or declare the macro `stmt`.",
    },
    Entry {
        code: "SWR4013",
        level: Level::Warning,
        title: "Statement or item macro used where an expression is read",
        explanation: "The macro is declared `stmt` or `item`, so it is not expanded inside an expression. Its name is left as written, and TypeScript will report it.",
        bad: "export syntax foo:stmt {\n  rule { foo $x:expr; } => { console.log($x); }\n}\nconst a = foo 1;\n",
        files: &[],
        fix: "Use it as a statement (`foo 1;`), or declare it `expr` if it should produce a value.",
    },
    Entry {
        code: "SWR4014",
        level: Level::Error,
        title: "Macro expands to itself",
        explanation: "A rule's expansion contains the same use again unchanged, so expansion would never end. The use is left as written.",
        bad: "export syntax loop:expr {\n  rule { loop($x:expr) } => { loop($x) }\n}\nconst a = loop(1);\n",
        files: &[],
        fix: "Make each step reduce its input, with a base rule that does not call the macro: `rule { loop() } => { 0 }`.",
    },
    Entry {
        code: "SWR4015",
        level: Level::Error,
        title: "Expansion reached a resource limit",
        explanation: "Expanding this use passed the nesting-depth limit (or another budget: steps, output size). Usually a recursive macro grows its input instead of shrinking it. Nothing further inside it was expanded.",
        bad: "export syntax grow:expr {\n  rule { grow($x:expr) } => { grow([$x]) }\n}\nconst a = grow(1);\n",
        files: &[],
        fix: "Make the recursion reduce its input and stop at a base case.",
    },
    Entry {
        code: "SWR4016",
        level: Level::Error,
        title: "Macro defined inside a block",
        explanation: "Definitions are read at module level only. One written inside a function or block is not processed.",
        bad: "function f() {\n  syntax foo:expr {\n    rule { foo() } => { 1 }\n  }\n}\n",
        files: &[],
        fix: "Move the definition to the top level, or to a macro module imported `for syntax`.",
    },
    Entry {
        code: "SWR4017",
        level: Level::Warning,
        title: "Macro used before its definition",
        explanation: "A macro is visible only to the code after its definition, so this earlier use is left as written.",
        bad: "const a = foo();\nexport syntax foo:expr {\n  rule { foo() } => { 1 }\n}\n",
        files: &[],
        fix: "Move the definition above the use, or into a macro module imported `for syntax`.",
    },
    Entry {
        code: "SWR4018",
        level: Level::Error,
        title: "Syntax parameter used outside `#parameterize`",
        explanation: "A `syntax parameter` (such as `%` in a pipe) has no rules of its own: it means something only inside a macro's `#parameterize(p = value) { ... }`. Written elsewhere it means nothing.",
        bad: "export syntax parameter (%):expr;\nconst b = %;\n",
        files: &[],
        fix: "Use it only inside the body of the macro that parameterizes it, for example `1 |> % + 1` with a pipe operator.",
    },
    Entry {
        code: "SWR4019",
        level: Level::Error,
        title: "`#parameterize` names something that is not a syntax parameter",
        explanation: "`#parameterize(name = value)` can only give meaning to a name declared with `syntax parameter` and visible where the template is defined.",
        bad: "export syntax foo:expr {\n  rule { foo($x:expr) } => { #parameterize(bar = { 1 }) { $x } }\n}\nconst a = foo(1);\n",
        files: &[],
        fix: "Declare it first (`export syntax parameter (bar):expr;`), and import it `for syntax` where the template lives.",
    },
    Entry {
        code: "SWR4020",
        level: Level::Error,
        title: "Could not read an operator operand as an expression",
        explanation: "A user-defined operator's operand group could not be read as an expression, so the operators in it were not tried.",
        bad: "export operator (+++):expr {\n  fixity infix;\n  associativity left;\n  precedence 10;\n  rule { $a:expr +++ $b:expr } => { $a + $b }\n}\nconst b = (1 +++ );\n",
        files: &[],
        fix: "Complete the expression: `const b = (1 +++ 2);`.",
    },
    Entry {
        code: "SWR4021",
        level: Level::Error,
        title: "Operator left unexpanded",
        explanation: "The expression around a user-defined operator could not be read, so none of the operator's rules were tried and it stays in the output as written.",
        bad: "export operator (+++):expr {\n  fixity infix;\n  associativity left;\n  precedence 10;\n  rule { $a:expr +++ $b:expr } => { $a + $b }\n}\nconst b = 1 +++ + ;\n",
        files: &[],
        fix: "Give the operator complete operands: `const b = 1 +++ 2;`.",
    },
    Entry {
        code: "SWR4022",
        level: Level::Error,
        title: "Required syntax parameter is never used",
        explanation: "The macro gives a syntax parameter a meaning with `#parameterize(required p = ...)`, which means the body must use `p`. This body does not.",
        bad: "export syntax parameter (%):expr;\nexport operator (|>):expr {\n  fixity infix;\n  associativity right;\n  precedence 20;\n  rule { $value:expr |> $body:expr } => {\n    #let(topic = $value) {\n      #parameterize(required % = topic) { $body }\n    }\n  }\n}\nconst a = 1 |> foo + 1;\n",
        files: &[],
        fix: "Use the parameter in the body (`1 |> % + 1`), or drop `required` from the macro.",
    },
    Entry {
        code: "SWR4024",
        level: Level::Warning,
        title: "Macro name written on its own",
        explanation: "A macro is a compile-time name. Written as a bare value (not invoked), it is copied into the output as is, where TypeScript will not find it.",
        bad: "export syntax foo:expr {\n  rule { foo() } => { 1 }\n}\nconst a = foo;\n",
        files: &[],
        fix: "Invoke it the way a rule expects (`const a = foo();`), or rename the value so it does not collide with the macro.",
    },
    Entry {
        code: "SWR4025",
        level: Level::Warning,
        title: "Item could not be read; passed through unexpanded",
        explanation: "The reader could not read this top-level item (for example legacy `let name = macro { ... }` syntax). It is copied as written, and no macro inside it was expanded.",
        bad: "let go = macro {\n  rule { $x } => { $x }\n}\nlet ok = 1;\n",
        files: &[],
        fix: "Rewrite it in current syntax: `export syntax go:expr { rule { go($x:expr) } => { $x } }`.",
    },
    Entry {
        code: "SWR4990",
        level: Level::Error,
        title: "Internal expander error",
        explanation: "The expander panicked while expanding this use. This is a bug in tsc-ext, not in your file. The file is passed through unexpanded from that point.",
        bad: "",
        files: &[],
        fix: "Report it with the file (`tsc-ext expand <file> --trace` output helps). Rewriting the use differently usually works around it.",
    },
    Entry {
        code: "SWR5002",
        level: Level::Error,
        title: "Macro module not found",
        explanation: "The module of a `for syntax` import does not resolve from this file. Macro modules are resolved like relative imports, by path, before TypeScript runs.",
        bad: "import { foo } from \"./missing-module.sts\" for syntax;\nconst a = 1;\n",
        files: &[],
        fix: "Fix the path (`./macros.sts`), relative to the importing file.",
    },
    Entry {
        code: "SWR5004",
        level: Level::Error,
        title: "Macro module does not export that name",
        explanation: "The `for syntax` import names a macro the module does not `export`.",
        bad: "import { nothere } from \"./m.sts\" for syntax;\nconst a = 1;\n",
        files: &[("m.sts", TWICE)],
        fix: "Import a name the module exports (`import { twice } from \"./m.sts\" for syntax;`), or add `export` to its definition.",
    },
    Entry {
        code: "SWR5005",
        level: Level::Error,
        title: "Macro module import cycle",
        explanation: "Macro modules import each other `for syntax` in a cycle, so none of them can be loaded first.",
        bad: "import { two } from \"./b.sts\" for syntax;\nconst a = two();\n",
        files: &[
            (
                "a.sts",
                "import { two } from \"./b.sts\" for syntax;\nexport syntax one:expr {\n  rule { one() } => { 1 }\n}\n",
            ),
            (
                "b.sts",
                "import { one } from \"./a.sts\" for syntax;\nexport syntax two:expr {\n  rule { two() } => { 2 }\n}\n",
            ),
        ],
        fix: "Move the shared macros into a third module that both import.",
    },
    Entry {
        code: "SWR5007",
        level: Level::Error,
        title: "Local macro name bound twice",
        explanation: "Two `for syntax` imports (or one import twice) bind the same local name.",
        bad: "import { twice, twice } from \"./m.sts\" for syntax;\nconst a = twice(1);\n",
        files: &[("m.sts", TWICE)],
        fix: "Import each name once, or rename one with `as`.",
    },
];

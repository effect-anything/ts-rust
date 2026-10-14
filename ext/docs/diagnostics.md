# Sweetener diagnostics

<!-- Generated from crates/ts_ext_sweetener/src/catalog.rs by `TSC_EXT_UPDATE=1 cargo test -p ts_ext_sweetener --test catalog`. Do not edit. -->

Every code the Sweetener expander reports. `tsc-ext explain <code>` prints one entry.
Diagnostics read `file:line:col: error SWRnnnn: message`. Codes 1xxx come from the reader (delimiters and literals), 2xxx from a macro definition's patterns and templates, 3xxx from binding contracts, 4xxx from expanding a use, and 5xxx from `for syntax` imports.

- [SWR1001](#swr1001) Unterminated string, template or regular-expression literal
- [SWR1002](#swr1002) Unexpected closing delimiter
- [SWR1003](#swr1003) Missing closing delimiter
- [SWR2001](#swr2001) Malformed macro definition
- [SWR2002](#swr2002) Malformed pattern
- [SWR2003](#swr2003) Unknown syntax category
- [SWR2004](#swr2004) Capture bound more than once
- [SWR2005](#swr2005) Alternatives bind different captures
- [SWR2006](#swr2006) Repetition can match without consuming input
- [SWR2008](#swr2008) Unresolved syntax class
- [SWR2009](#swr2009) Left-recursive syntax class
- [SWR2010](#swr2010) Invalid refinement
- [SWR2011](#swr2011) Template refers to an unknown capture
- [SWR2012](#swr2012) Capture has no such field
- [SWR2013](#swr2013) Capture used at the wrong repetition depth
- [SWR2014](#swr2014) Template repetition has no driving capture
- [SWR2015](#swr2015) Template repetition has incompatible driving captures
- [SWR2016](#swr2016) Malformed template
- [SWR2017](#swr2017) Invalid argument for a template operation
- [SWR2018](#swr2018) Unknown template operation
- [SWR2019](#swr2019) Malformed `for syntax` import
- [SWR3002](#swr3002) Malformed binding contract
- [SWR3003](#swr3003) Invalid binding-contract binder
- [SWR3004](#swr3004) Binding-contract repetitions are not alignable
- [SWR3005](#swr3005) Binding space not allowed for this macro category
- [SWR4001](#swr4001) No rule of the macro accepted this use
- [SWR4002](#swr4002) `shadows core` on something that is not a core form
- [SWR4003](#swr4003) Import asks to shadow a core form the definition does not allow
- [SWR4006](#swr4006) Invalid operator definition
- [SWR4008](#swr4008) Unknown macro context
- [SWR4009](#swr4009) Binding literal cannot resolve its path
- [SWR4010](#swr4010) Macro defined twice in one module
- [SWR4011](#swr4011) Expression macro expanded to something that is not one expression
- [SWR4013](#swr4013) Statement or item macro used where an expression is read
- [SWR4014](#swr4014) Macro expands to itself
- [SWR4015](#swr4015) Expansion reached a resource limit
- [SWR4016](#swr4016) Macro defined inside a block
- [SWR4017](#swr4017) Macro used before its definition
- [SWR4018](#swr4018) Syntax parameter used outside `#parameterize`
- [SWR4019](#swr4019) `#parameterize` names something that is not a syntax parameter
- [SWR4020](#swr4020) Could not read an operator operand as an expression
- [SWR4021](#swr4021) Operator left unexpanded
- [SWR4022](#swr4022) Required syntax parameter is never used
- [SWR4024](#swr4024) Macro name written on its own
- [SWR4025](#swr4025) Item could not be read; passed through unexpanded
- [SWR4990](#swr4990) Internal expander error
- [SWR5002](#swr5002) Macro module not found
- [SWR5004](#swr5004) Macro module does not export that name
- [SWR5005](#swr5005) Macro module import cycle
- [SWR5007](#swr5007) Local macro name bound twice

## SWR1001

**Unterminated string, template or regular-expression literal** (error)

The reader reached the end of the line (or file) inside a literal. Sweetener reads the whole file as tokens before any macro runs, so an open quote swallows the rest of the file and no macro after it can be found.

Triggers it:

```ts
const s = "open;
const x = 1;
```

Fix:

Close the literal: `const s = "open";`.

## SWR1002

**Unexpected closing delimiter** (error)

A `)`, `]` or `}` has no matching opener. Sweetener groups tokens by delimiters before it expands anything, so this is reported by the reader at that byte, not as a TypeScript parse error.

Triggers it:

```ts
const a = f(1));
```

Fix:

Remove the extra closer, or add the opener it belongs to: `const a = f(1);`.

## SWR1003

**Missing closing delimiter** (error)

A `(`, `[` or `{` is never closed before the end of the file. The reader reports it at the opener.

Triggers it:

```ts
function f() {
  return 1;
```

Fix:

Close the group: add the missing `}` after `return 1;`.

## SWR2001

**Malformed macro definition** (error)

A `syntax` or `operator` definition is missing a part the reader needs: the name and category (`syntax name:category`), the body `{ ... }`, or a rule's pattern group `rule { ... }`.

Triggers it:

```ts
export syntax twice {
  rule { twice($x:expr) } => { [$x, $x] }
}
```

Fix:

Write the category after the name: `export syntax twice:expr { ... }`. Categories are `expr`, `stmt` and `item`.

## SWR2002

**Malformed pattern** (error)

A rule pattern uses `$` without a capture (`$name:class`) or a repetition `$( ... )` without `*`, `+` or `?` after it.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($) } => { 1 }
}
```

Fix:

Name the capture and its class: `rule { foo($x:expr) } => { $x }`, or close a repetition with `*`, `+` or `?`.

## SWR2003

**Unknown syntax category** (error)

The category after the macro name is not one Sweetener knows. It decides where the macro may be written and what its expansion must be.

Triggers it:

```ts
export syntax foo:banana {
  rule { foo() } => { 1 }
}
```

Fix:

Use `expr` (an expression), `stmt` (a statement) or `item` (a module-level declaration): `export syntax foo:expr { ... }`.

## SWR2004

**Capture bound more than once** (error)

Two captures in one pattern have the same name, so the template could not tell which one `$x` means.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:expr, $x:expr) } => { $x }
}
```

Fix:

Give each capture its own name: `rule { foo($a:expr, $b:expr) } => { $a + $b }`.

## SWR2005

**Alternatives bind different captures** (error)

The branches of a `|` alternative must bind the same captures with the same shape; otherwise a capture the template uses would be missing when the other branch matched.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo(a $x:expr | b) } => { 1 }
}
```

Fix:

Bind the same captures in every branch, or split the alternatives into separate rules: `rule { foo(a $x:expr) } => { $x }` and `rule { foo(b) } => { 1 }`.

## SWR2006

**Repetition can match without consuming input** (error)

A `*` or `+` repetition whose body can match nothing would loop forever, so the definition is refused.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($($($x:expr)?)*) } => { 1 }
}
```

Fix:

Make every repetition consume at least one token: `rule { foo($($x:expr),*) } => { [$($x),*] }`.

## SWR2008

**Unresolved syntax class** (error)

A capture names a class (`$x:class`) that is neither built in (`expr`, `stmt`, `ident`, `binding`, `type`, `tt`, ...) nor a `syntax class` defined in or imported into this file.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:nosuch) } => { $x }
}
```

Fix:

Use a built-in class (`$x:expr`), or define `syntax class nosuch { ... }` above the macro.

## SWR2009

**Left-recursive syntax class** (error)

A `syntax class` refers to itself before consuming any token, so matching it would recurse without end.

Triggers it:

```ts
syntax class loop {
  fields { v: tt; }
  rule { $l:loop + $v:tt }
}
```

Fix:

Consume a token before the recursive reference (`rule { ( $l:loop ) }`), or use a repetition: `rule { $($v:tt)+ }`.

## SWR2010

**Invalid refinement** (error)

A `refine` clause names a capture the rule does not bind, or uses a predicate Sweetener does not know.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:expr) }
  refine $y length equal 1;
  => { $x }
}
```

Fix:

Refine a capture of the same rule: `refine $x length equal 1;`.

## SWR2011

**Template refers to an unknown capture** (error)

The template uses `$name`, but the rule's pattern binds no capture of that name.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:expr) } => { $y }
}
const a = foo(1);
```

Fix:

Use a capture the pattern binds: `=> { $x }`.

## SWR2012

**Capture has no such field** (error)

`$capture.field` names a field that the capture's syntax class does not declare in its `fields { ... }`.

Triggers it:

```ts
syntax class Pair {
  fields { left: expr; right: expr; }
  rule { [$left:expr, $right:expr] }
}
export syntax foo:expr {
  rule { foo($p:Pair) } => { $p.nope }
}
```

Fix:

Use a declared field: `=> { $p.left }`.

## SWR2013

**Capture used at the wrong repetition depth** (error)

A capture bound inside `$( ... )*` holds a list. The template must use it inside a repetition of the same depth.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($($x:expr),*) } => { [$x] }
}
```

Fix:

Repeat it as the pattern does: `=> { [$($x),*] }`.

## SWR2014

**Template repetition has no driving capture** (error)

A `$( ... )*` in a template repeats once per element of the repeated captures it contains. This one contains none, so its length is unknown.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:expr) } => { [$(1),*] }
}
```

Fix:

Repeat over a repeated capture (`rule { foo($($x:expr),*) } => { [$($x),*] }`), or drop the repetition.

## SWR2015

**Template repetition has incompatible driving captures** (error)

One template repetition uses captures from different pattern repetitions, which can have different lengths.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($($x:expr),* ; $($y:expr),*) } => { [$($x + $y),*] }
}
```

Fix:

Repeat each list on its own (`=> { [$($x),*, $($y),*] }`), or capture pairs in one repetition: `rule { foo($($x:expr : $y:expr),*) }`.

## SWR2016

**Malformed template** (error)

A template repetition `$( ... )` has no `*` or `+` after it, or an `#if` is not exactly `#if(present $capture)`.

Triggers it:

```ts
export syntax twice:expr {
  rule { twice($($x:expr),*) } => { [$($x),] }
}
```

Fix:

Close the repetition with its separator and `*`: `=> { [$($x),*] }`. An optional capture is tested with `#if(present $x) { ... } #else { ... }`.

## SWR2017

**Invalid argument for a template operation** (error)

A template operation such as `#fresh("name")`, `#capture($x)`, `#callsite($x)` or `#definition($x)` got an argument of the wrong kind: `#fresh` takes one identifier-like string, the scope operations take a capture.

Triggers it:

```ts
export syntax w:stmt {
  rule { w $a:ident; } => { #capture(result) = 1; }
}
w x;
```

Fix:

Pass a capture: `=> { #capture($a) = 1; }`, and give `#fresh` one word: `#fresh("tmp")`.

## SWR2018

**Unknown template operation** (error)

The template calls `#name(...)`, but there is no such operation. Known ones include `#if`, `#else`, `#let`, `#parameterize`, `#fold`, `#fresh`, `#join`, `#count`, `#trim`, `#core`, `#capture`, `#callsite` and `#definition`.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:expr) } => { #nosuch($x) }
}
const a = foo(1);
```

Fix:

Use a known operation, or write the code directly: `=> { $x }`.

## SWR2019

**Malformed `for syntax` import** (error)

A compile-time import must read `import { names } from "module" for syntax;`, optionally followed by `shadows core`. Anything else after `for syntax` is refused.

Triggers it:

```ts
import { foo } from "./x.sts" for syntax as;
const a = 1;
```

Fix:

`import { foo } from "./x.sts" for syntax;`

## SWR3002

**Malformed binding contract** (error)

A `bind $name in <scope> as <kind> <space>;` clause names a capture the rule does not bind, or is missing a part (the scope, `as`, the kind or the space).

Triggers it:

```ts
export syntax a:expr {
  rule { a $x:expr }
  bind $y in following as lexical value;
  => { $x }
}
```

Fix:

Bind a capture of this rule and write every part: `bind $name in following as lexical value;`.

## SWR3003

**Invalid binding-contract binder** (error)

The binder of a `bind` clause must be an `ident`, `binding` or class capture. An expression capture does not introduce a name.

Triggers it:

```ts
export syntax c:stmt {
  rule { c $x:expr $v:expr }
  bind $x in $v as lexical value;
  => { $v; }
}
```

Fix:

Capture the name as a binding: `rule { c $x:binding $v:expr }`.

## SWR3004

**Binding-contract repetitions are not alignable** (error)

A `sequential` contract pairs each binder with the scope that follows it, so the binder and scope captures must repeat together.

Triggers it:

```ts
export syntax foo:stmt {
  rule { foo $a:binding in $b:expr ; }
  bind $a in $b as sequential value;
  => { 1; }
}
```

Fix:

Use `lexical` for a single binder (`bind $a in $b as lexical value;`), or repeat binder and scope in one repetition.

## SWR3005

**Binding space not allowed for this macro category** (error)

An expression macro can bind values only. Types (and other spaces) can be bound by statement and item macros.

Triggers it:

```ts
export syntax b:expr {
  rule { b $x:ident $v:expr }
  bind $x in $v as lexical type;
  => { $v }
}
```

Fix:

Bind a value (`as lexical value`), or make the macro a `stmt`/`item` macro.

## SWR4001

**No rule of the macro accepted this use** (error)

The macro is known here, but none of its rules matched the tokens at this use. The message says what the closest rule expected (its `expect` text, or the context it needs); a related location points where that rule stopped. `tsc-ext expand --trace` shows every rule tried and why it failed.

Triggers it:

```ts
export syntax unless:stmt {
  rule { unless ($c:expr) { $($b:stmt)* } } => { if (!($c)) { $($b)* } }
}
unless x { f(); }
```

Fix:

Write the use the way a rule expects (`unless (x) { f(); }`), or add a rule for this shape.

## SWR4002

**`shadows core` on something that is not a core form** (error)

`shadows core` lets a macro replace a built-in TypeScript form such as `function`. The macro's name is not one of those forms, so there is nothing to shadow.

Triggers it:

```ts
export syntax hello:expr shadows core {
  rule { hello } => { 1 }
}
const x = hello;
```

Fix:

Drop `shadows core` (`export syntax hello:expr { ... }`); it is only for core forms.

## SWR4003

**Import asks to shadow a core form the definition does not allow** (error)

`import { name } ... for syntax shadows core` requests that the imported macro replace a core form, but its definition was not written with `shadows core`.

`fn.sts`:

```ts
export syntax function:expr {
  rule { function($x:ident: $t:type) { $e:expr } } => { (($x: $t) => $e) }
}
```

Triggers it:

```ts
import { function } from "./fn.sts" for syntax shadows core;
const f = function (x: number) { x };
```

Fix:

Write `shadows core` on the definition (`export syntax function:expr shadows core { ... }`), or import without it.

## SWR4006

**Invalid operator definition** (error)

An `operator` needs a `fixity` (`prefix`, `infix` or `postfix`), and an infix operator needs `associativity` (`left`, `right` or `none`) and a `precedence`. The message names the missing or invalid setting.

Triggers it:

```ts
export operator (+++):expr {
  fixity infix;
  precedence 5;
  rule { $a:expr +++ $b:expr } => { $a }
}
```

Fix:

Add the missing setting: `associativity left;`.

## SWR4008

**Unknown macro context** (error)

A rule's `context` clause names a context Sweetener does not track. Known contexts are `async` and `generator`.

Triggers it:

```ts
export syntax odd:expr {
  rule { odd() }
  context weird;
  => { 1 }
}
```

Fix:

`context async;` or `context generator;`, or remove the clause.

## SWR4009

**Binding literal cannot resolve its path** (error)

`literal path as Name;` must name a real global path (such as `globalThis.NaN`) for the spelling it rewrites.

Triggers it:

```ts
export syntax foo:expr {
  literal nosuch.thing as NaN;
  rule { foo() } => { NaN }
}
```

Fix:

`literal globalThis.NaN as NaN;`

## SWR4010

**Macro defined twice in one module** (error)

Two definitions in one file have the same name. Rules for one macro belong in one definition.

Triggers it:

```ts
syntax foo:expr {
  rule { foo() } => { 1 }
}
syntax foo:expr {
  rule { foo() } => { 2 }
}
```

Fix:

Merge the rules into one definition, or rename one macro.

## SWR4011

**Expression macro expanded to something that is not one expression** (error)

A macro declared `expr` produced a statement (or several pieces). Its uses stand where an expression is read, so the expansion would not parse there.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo() } => { if (x) {} }
}
const a = foo();
```

Fix:

Make the template one expression (`=> { x ? 1 : 0 }`), or declare the macro `stmt`.

## SWR4013

**Statement or item macro used where an expression is read** (warning)

The macro is declared `stmt` or `item`, so it is not expanded inside an expression. Its name is left as written, and TypeScript will report it.

Triggers it:

```ts
export syntax foo:stmt {
  rule { foo $x:expr; } => { console.log($x); }
}
const a = foo 1;
```

Fix:

Use it as a statement (`foo 1;`), or declare it `expr` if it should produce a value.

## SWR4014

**Macro expands to itself** (error)

A rule's expansion contains the same use again unchanged, so expansion would never end. The use is left as written.

Triggers it:

```ts
export syntax loop:expr {
  rule { loop($x:expr) } => { loop($x) }
}
const a = loop(1);
```

Fix:

Make each step reduce its input, with a base rule that does not call the macro: `rule { loop() } => { 0 }`.

## SWR4015

**Expansion reached a resource limit** (error)

Expanding this use passed the nesting-depth limit (or another budget: steps, output size). Usually a recursive macro grows its input instead of shrinking it. Nothing further inside it was expanded.

Triggers it:

```ts
export syntax grow:expr {
  rule { grow($x:expr) } => { grow([$x]) }
}
const a = grow(1);
```

Fix:

Make the recursion reduce its input and stop at a base case.

## SWR4016

**Macro defined inside a block** (error)

Definitions are read at module level only. One written inside a function or block is not processed.

Triggers it:

```ts
function f() {
  syntax foo:expr {
    rule { foo() } => { 1 }
  }
}
```

Fix:

Move the definition to the top level, or to a macro module imported `for syntax`.

## SWR4017

**Macro used before its definition** (warning)

A macro is visible only to the code after its definition, so this earlier use is left as written.

Triggers it:

```ts
const a = foo();
export syntax foo:expr {
  rule { foo() } => { 1 }
}
```

Fix:

Move the definition above the use, or into a macro module imported `for syntax`.

## SWR4018

**Syntax parameter used outside `#parameterize`** (error)

A `syntax parameter` (such as `%` in a pipe) has no rules of its own: it means something only inside a macro's `#parameterize(p = value) { ... }`. Written elsewhere it means nothing.

Triggers it:

```ts
export syntax parameter (%):expr;
const b = %;
```

Fix:

Use it only inside the body of the macro that parameterizes it, for example `1 |> % + 1` with a pipe operator.

## SWR4019

**`#parameterize` names something that is not a syntax parameter** (error)

`#parameterize(name = value)` can only give meaning to a name declared with `syntax parameter` and visible where the template is defined.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo($x:expr) } => { #parameterize(bar = { 1 }) { $x } }
}
const a = foo(1);
```

Fix:

Declare it first (`export syntax parameter (bar):expr;`), and import it `for syntax` where the template lives.

## SWR4020

**Could not read an operator operand as an expression** (error)

A user-defined operator's operand group could not be read as an expression, so the operators in it were not tried.

Triggers it:

```ts
export operator (+++):expr {
  fixity infix;
  associativity left;
  precedence 10;
  rule { $a:expr +++ $b:expr } => { $a + $b }
}
const b = (1 +++ );
```

Fix:

Complete the expression: `const b = (1 +++ 2);`.

## SWR4021

**Operator left unexpanded** (error)

The expression around a user-defined operator could not be read, so none of the operator's rules were tried and it stays in the output as written.

Triggers it:

```ts
export operator (+++):expr {
  fixity infix;
  associativity left;
  precedence 10;
  rule { $a:expr +++ $b:expr } => { $a + $b }
}
const b = 1 +++ + ;
```

Fix:

Give the operator complete operands: `const b = 1 +++ 2;`.

## SWR4022

**Required syntax parameter is never used** (error)

The macro gives a syntax parameter a meaning with `#parameterize(required p = ...)`, which means the body must use `p`. This body does not.

Triggers it:

```ts
export syntax parameter (%):expr;
export operator (|>):expr {
  fixity infix;
  associativity right;
  precedence 20;
  rule { $value:expr |> $body:expr } => {
    #let(topic = $value) {
      #parameterize(required % = topic) { $body }
    }
  }
}
const a = 1 |> foo + 1;
```

Fix:

Use the parameter in the body (`1 |> % + 1`), or drop `required` from the macro.

## SWR4024

**Macro name written on its own** (warning)

A macro is a compile-time name. Written as a bare value (not invoked), it is copied into the output as is, where TypeScript will not find it.

Triggers it:

```ts
export syntax foo:expr {
  rule { foo() } => { 1 }
}
const a = foo;
```

Fix:

Invoke it the way a rule expects (`const a = foo();`), or rename the value so it does not collide with the macro.

## SWR4025

**Item could not be read; passed through unexpanded** (warning)

The reader could not read this top-level item (for example legacy `let name = macro { ... }` syntax). It is copied as written, and no macro inside it was expanded.

Triggers it:

```ts
let go = macro {
  rule { $x } => { $x }
}
let ok = 1;
```

Fix:

Rewrite it in current syntax: `export syntax go:expr { rule { go($x:expr) } => { $x } }`.

## SWR4990

**Internal expander error** (error)

The expander panicked while expanding this use. This is a bug in tsc-ext, not in your file. The file is passed through unexpanded from that point.

Fix:

Report it with the file (`tsc-ext expand <file> --trace` output helps). Rewriting the use differently usually works around it.

## SWR5002

**Macro module not found** (error)

The module of a `for syntax` import does not resolve from this file. Macro modules are resolved like relative imports, by path, before TypeScript runs.

Triggers it:

```ts
import { foo } from "./missing-module.sts" for syntax;
const a = 1;
```

Fix:

Fix the path (`./macros.sts`), relative to the importing file.

## SWR5004

**Macro module does not export that name** (error)

The `for syntax` import names a macro the module does not `export`.

`m.sts`:

```ts
export syntax twice:expr {
  rule { twice($x:expr) } => { [$x, $x] }
}
```

Triggers it:

```ts
import { nothere } from "./m.sts" for syntax;
const a = 1;
```

Fix:

Import a name the module exports (`import { twice } from "./m.sts" for syntax;`), or add `export` to its definition.

## SWR5005

**Macro module import cycle** (error)

Macro modules import each other `for syntax` in a cycle, so none of them can be loaded first.

`a.sts`:

```ts
import { two } from "./b.sts" for syntax;
export syntax one:expr {
  rule { one() } => { 1 }
}
```

`b.sts`:

```ts
import { one } from "./a.sts" for syntax;
export syntax two:expr {
  rule { two() } => { 2 }
}
```

Triggers it:

```ts
import { two } from "./b.sts" for syntax;
const a = two();
```

Fix:

Move the shared macros into a third module that both import.

## SWR5007

**Local macro name bound twice** (error)

Two `for syntax` imports (or one import twice) bind the same local name.

`m.sts`:

```ts
export syntax twice:expr {
  rule { twice($x:expr) } => { [$x, $x] }
}
```

Triggers it:

```ts
import { twice, twice } from "./m.sts" for syntax;
const a = twice(1);
```

Fix:

Import each name once, or rename one with `as`.

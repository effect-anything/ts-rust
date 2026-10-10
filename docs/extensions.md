# Extensions

Our syntax changes stay out of the typescript-go port. Upstream sync should
only see two calls. The work itself lives in our own files.

## Two cuts

```text
foreign extension
  -> content mapper process
  -> ordinary TypeScript + span map

ordinary source
  -> parse
  -> ext::precheck::apply
  -> check
  -> emit if ext::precheck::emits_javascript
  -> ext::precheck::emit_javascript
```

A content mapper owns the runtime output of a file whose extension it
registered. `.ts` and `.tsx` cannot be registered: the host would send every
TypeScript file through that process.

`precheck` runs after parse and after a content mapper. It may rewrite the
text and attach a span map. It does not own runtime output, so emit still
belongs to this compiler. Another rewrite is another call inside
`ext::precheck::apply`, not another branch in the port.

A plugin that does not use this compiler calls the same transforms itself,
then hands ordinary TypeScript to its own pipeline. Runtime wrappers that
check must not see belong to that plugin's build phase, not to `precheck`.

## Where the code lives

| path | role |
| --- | --- |
| `crates/ts_goport/src/ext/` | Internal rewrites. Not part of the port. |
| `crates/ts_ext_syntax/` | External mapper and plugin session. |
| `crates/ext_syntax_node/` | Node-API for the plugin session. |

Port files that call the hooks: the compiler host, the content-mapper
result parser, the project parse cache, and the emit decision. Those calls
should stay one line. Do not put a rewrite, an extension list, or a product
name in the port.

## What this can do

The split is the capability. Check sees legal TypeScript. The author can
still write something else, as long as one of the two cuts turns it into
TypeScript before check. Diagnostics use the span map, so they point at the
author's text. A build step may wrap the checked text again for a runtime.
Check does not see that wrap.

A form the parser already accepts belongs in `precheck`. A tagged template,
a directive string, or a call can become another call. Check then uses
ordinary overloads. For example:

- `2m + 50cm` becomes `meters(2) + centimeters(50)`.
- `` sql`select ${id}` `` stays a tagged template, but its contents are
  checked as a query.
- `user as Admin` becomes `requireRole(user, "admin")`.
- `use cache(key) { load() }` becomes `cache(key, () => load())`.

A form the parser rejects belongs in a content mapper, on its own
extension. A `.sql` file, or a macro file, is expanded to TypeScript first.
If that TypeScript still contains a form from the list above, `precheck`
runs on the expanded text. The two maps compose.

The build phase is only for output the checker must not see: a runtime
wrapper, a metadata table, or a lowered call. It does not change types.

## What this cannot do

It cannot change the parser, the binder, or the checker. A new token, a new
expression form, or a new type rule does not belong here. If the parser
rejects the file, register an extension and expand it outside. Do not
register `.ts` or `.tsx` to catch it.

It cannot own the runtime of a content-mapped file. That output belongs to
the mapper or the plugin. Internal `precheck` rewrites still emit through
this compiler.

It does not load a runtime by itself. A rewritten call needs a declaration
the checker can see, and an implementation the program imports. The hook
does not inject either into every file.

## Check hook

The port does not grow a second mapper, a second LSP path, or an extension
list. Content mappers already run when tsconfig registers an extension.
`precheck` is the only internal call, and it runs after that mapping.

LSP already maps ranges through a span map attached to the file. A rewrite
that sets that map does not add a converter in `ls/` or `lsp/`. A file the
rewrite does not change gets no marker.



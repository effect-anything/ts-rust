# Check column

`cargo run -p ts_ext_syntax --example check_cases` writes `ours.check.txt`
next to each captured case. It runs `target/debug/tsgo --noEmit` on the
expanded entry. `expected.*` files are not part of the program.

Third-party import errors (TS2305, TS2307, TS2595, TS2614, TS2694, TS2749)
are removed. Those are stub gaps. Syntax errors stay.

Last run: 95 cases. TypeGPU 20/20 clean. Sweetener 71/75 clean.

The four Sweetener failures are checker disagreements with their expanded
text, not missing deps:

- `fixtures/acceptance/playground/new-language`: their expander prints
  `module List`. tsgo reports TS1540 and wants `namespace`.
- `library-macros/effect-do` and `effect-service`: TS2347, an untyped call
  given a type argument.
- `vite-react/src/main`: TS2558, a call given one type argument where the
  stub has zero. This one may move if the react stub grows real signatures.

`ours.check.txt` prefixes a diagnostic with `original(line,col)` when the
virtual offset falls in a segment. Sweetener's bridge currently emits one
Atom for the whole file, so that prefix is the start of the original file,
not the operator or macro token. A finer map needs the source-map segments.
TypeGPU operator atoms can already point at the operator when a diagnostic
lands on the rewritten call. No TypeGPU case currently has a semantic
diagnostic, so that path is untested here.

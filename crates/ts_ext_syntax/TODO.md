# Extensions follow-up

Sweetener cases are flat directories under `crates/ts_ext_sweetener/cases`, one program per `cases.jsonl` row. `vendor/` is a gitignored pair of symlinks to local reference checkouts. The port does not read those trees. Discover and sync may.

## Diagnostics

- [x] A failed expansion returns no virtual TypeScript, so the parser does not see the Sweetener source
- [x] Other files stay separate transforms
- [x] The 16 `malformed` cases match the phrase in `expected.malformed.diagnostics.json`. A hole in `->` or `#` is not repaired into a call
- [x] `ext-syntax transform` prints `file.sts(line,col): error <code>: message`
- [ ] Diagnostic codes are not unified. The wrapper is done; the prefix is not. `report_message` keeps a message that already starts with `SWR` or `TS`. Reader errors are `SWR1002` and `SWR1003`. A missing template directive or a syntax import that cannot be loaded is `SWR4001`. "No rule for macro" is `TS4001`. `expected.malformed.diagnostics.json` uses both. The host format is `error TS<code>:`.

## Source map

- [x] Compose the Sweetener map and the operator rewrite into one map, in the content-mapper hook and in `TransformSession::check`
- [x] `__TYPEGPU_META__` is build output only. It has no span map and does not need one
- [x] Diagnostics and positions in generated text point at the author file
- [x] A macro atom covers the lines the macro produced
- [x] Tests assert offsets, not generated formatting

## LSP

- [x] Check and diagnostics read the span map the content-mapper hook attached
- [x] Definition on a macro atom lands on the author macro, not a template
- [x] Rename is refused for a generated or hygiene name. An author name outside the macro still maps 1:1

## One API

- [ ] Export only `new TransformSession({ typegpu: false | {...}, sweetener: false | {...} })`
- [ ] Keep Sweetener and TypeGPU internal
- [ ] `session.check({ filename, source, sweetener, typegpu })`
- [ ] `session.transform({ filename, source, ... })`
- [ ] `session.transformFile({ filePath, ... })` for the CLI and tests
- [ ] `source` does not read disk
- [ ] Both passes default on

## Expander: library forms and missing template behavior

Found while adding Effect atom macros to `ext/examples/vite`. The form pass runs before the imported macro, so a user template cannot replace these.

### Library-specific forms

The token walk should not know Effect, signals, or React. It does:

- `sweetener_apps.rs` `expand_apps` matches `error`, `service`, `effect name()`, `handle`, `gen`, `query`, `table`, and the React hook spellings by keyword. `expand_service` prints `Context.Tag` (Effect 3). A macro file that says `Context.Service` is never used.
- `sweetener_forms.rs` turns `state name = initial` into `createSignal` / `.get()` / `.set()` unless the file text contains `useState`. `effect { ... }` becomes `createEffect(() => { ... })` with no import check.
- `expand_apps` also switches on the substrings `useState`, `latestRef`, and `shadows core`.

### Not implemented, so some macros cannot be written

- [x] `#metavar` is a hole of the generated rule (`Tpl::Metavar`). `#trim` and `#index` render too.
- [x] `macro_call_position` accepts `=`, `+=`, and the other assignment spellings, plus a bare name and a comma. A closer such as `)` is not a call.
- [x] A relative runtime import is rebased from the macro file onto the file that imported it. Syntax imports inside a macro module are loaded too.
- [ ] Seven captured cases still differ: memoized, function-shadow, vite-react main, both multi-part-methods, both adt. The form pass still owns service, query, table, and memoized where the compiled rule does not match.

### Goal

No `diff.txt`. Every captured case expands through the macro the author imported. The expander does not special-case a library, a runtime name, or a snippet of source. `capture_ours` deletes `diff.txt` only when the expansion and `theirs` are the same shape.

`cargo test -p ts_ext_sweetener --tests` is green and does not check this. `every_local_macro_case_matches_theirs` skips a case that has no separate `macros.sts`. The 14 `diff.txt` files below are those cases: the macro is inside `input.sts`, and `structure::same` still fails.

- React memoization: `examples-react-memoization-src-fine-jsx-implementation`, `examples-react-memoization-src-function-shadow`, `examples-react-memoization-src-memoized`. Missing `import { c as _c } from "react/compiler-runtime"`.
- Vite React: `examples-vite-react-src-main`, `examples-vite-react-src-react-hooks`. Expanding the macro module drops the React import. The hooks `ours` is empty.
- Playground shape: `fixtures-acceptance-playground-adt`, `fixtures-acceptance-playground-adt-hygiene`. The union and constructors are present, but `<T,>`, parentheses, and trailing commas are not the `theirs` tree.
- Playground channels: `fixtures-acceptance-playground-csp`, `fixtures-acceptance-playground-csp-hygiene`. `2 -> table` and `received <- table` stay unexpanded instead of `yield put` / `yield take`.
- Playground hygiene: `fixtures-acceptance-playground-currying-hygiene`, `fixtures-acceptance-playground-do-notation-hygiene`, `fixtures-acceptance-playground-multi-part-methods`, `fixtures-acceptance-playground-multi-part-methods-hygiene`, `fixtures-acceptance-playground-new-language-hygiene`. The fresh names or the generated shape differ from `theirs`.

Eleven other `diff.txt` files are already deleted in the worktree. Those cases match and are not this list.

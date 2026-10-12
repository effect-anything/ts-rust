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

The token walk should not know Effect, signals, or React. What it still does:

- `sweetener_apps.rs` `expand_apps` still matches `error`, `effect name()`, `handle`, and `gen` by keyword. `query`, `table`, and `memoized` expand through the imported macro. `service` does too: a macro that says `Context.Service` is the one that runs.
- `sweetener_forms.rs` no longer switches on `useState`, `latestRef`, or `shadows core`. `state` and `effect { ... }` are the imported macro.
- `match_runtime_import` still prefixes `import { c as _c } from "react/compiler-runtime"` when the file is the React compiler fixture and the macro module's import was not copied. The fixture itself expands through `emitCompilerFixture`.

### Not implemented, so some macros cannot be written

- [x] `#metavar` is a hole of the generated rule (`Tpl::Metavar`). `#trim` and `#index` render too.
- [x] `macro_call_position` accepts `=`, `+=`, and the other assignment spellings, plus a bare name and a comma. A closer such as `)` is not a call.
- [x] A relative runtime import is rebased from the macro file onto the file that imported it. Syntax imports inside a macro module are loaded too.
- [x] A class rule's `refine` is checked: `spelling equals`, `spelling starts-with-lowercase`, `delimiter parenthesis`, and `length equal N`. A failed guard tries the next rule of that class.
- [x] No `diff.txt` under `crates/ts_ext_sweetener/cases`. `query`, `table`, `memoized`, function-shadow, and both multi-part-methods expand through the imported macro.

### Goal

No `diff.txt`. Every captured case expands through the macro the author imported. The expander does not special-case a library, a runtime name, or a snippet of source. `capture_ours` deletes `diff.txt` only when the expansion and `theirs` are the same shape.

`every_local_macro_case_matches_theirs` checks each captured case, including a macro defined inside `input.sts`. The 14 cases this section used to list match. `error`, `effect name()`, `handle`, and `gen` are the keyword rewrites still in `expand_apps`.

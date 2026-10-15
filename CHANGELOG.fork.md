# Changelog

All notable changes on `wip` since `main` are documented here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/). Entries are unreleased and each one names the commit it comes from. Merge commits (`464c12ff6`, `aae68d5fe`, `30daf6c82`) carry the same diff as the branch they merge and are not listed again.

## [Unreleased]

### Added

#### Sweetener

- `crates/ts_ext_sweetener`: the Sweetener expander, its captured cases under `cases/`, and the check, discover, and sync scripts (`2af2b6fe2`, `9a8c38188`).
- `#metavar`, `#trim`, and `#index` render. Generated names match assignment and bare uses. A runtime import is rebased onto the file that imported it, and a macro defined in the file being expanded is compiled (`6d2e1de14`).
- A class rule's `refine` guard is checked (`spelling equals`, `spelling starts-with-lowercase`, `delimiter parenthesis`, `length equal N`). A failed guard tries the next rule of that class, so `query`, `table`, and `memoized` expand through the imported macro (`351698e3a`).
- `service` expands through its imported macro. Statement macros are not tried inside JSX text. A `#join` name stays the author binding, and a data constructor does not rename a field that shadows an outer `const` (`8cd15b053`).
- Captured cases for signals, channels, adt, Vite hooks, memoized, function-shadow, and both multi-part-methods cases expand through the imported macro. No `diff.txt` remains under `crates/ts_ext_sweetener/cases` (`6d2e1de14`, `8cd15b053`, `6c3d3dfb5`, `a40856c4a`, `351698e3a`).
- Examples under `ext/examples/sweetener` and the Vite example under `ext/examples/vite` (`2af2b6fe2`).

#### tsover

- `crates/ts_goport/src/ext/tsover.rs` (moved from `overload.rs`): operator rewrite for `"use tsover"` and `"use gpu"`, the operator table, prelude, and span-map segments (`1cd793bc5`, `cdb320d13`). `**` becomes `__tsover_pow` only in a `"use tsover"` file.
- Tests in `crates/ts_ext_typegpu/tests/tsover.rs` and the write-up in `ext/docs/tsover.md`. The example is `ext/examples/tsover` (`1cd793bc5`).

#### TypeGPU

- `crates/ts_goport/src/ext/typegpu.rs`: the TypeGPU emit hook. It hoists a marked function, wraps it with `__TYPEGPU_META__` (format version 2), and embeds the tinyest AST. A labelled `tgpu.*` resource constructor is wrapped with `__TYPEGPU_AUTONAME__` (`a680a4ec8`, `84f83180b`, `cdb320d13`).
- `embed_metadata_reports` reports a body the transpiler refuses and marks that body unprintable instead of embedding a wrong tree (`a3b497903`).
- Tests in `crates/ts_ext_typegpu/tests/typegpu.rs`, `ext/docs/typegpu.md`, and `ext/examples/typegpu` (`2af2b6fe2`, `a3b497903`).

#### ts_ext_syntax / LSP

- `crates/ts_ext_syntax`: the Node package and `ext-syntax` CLI. `transform` returns virtual text, a virtual extension, span-map tuples, and original-file diagnostics (`2af2b6fe2`, `54f7fd63e`).
- The content-mapper path runs Sweetener only. `tsc` runs TypeGPU after it, so a mapper does not also rewrite `"use gpu"` (`protocol.rs`, `30daf6c82`).
- Check and diagnostics read the span map the content-mapper hook attached. Definition on a macro atom lands on the author macro. Rename is refused for a generated or hygiene name (`TODO.md`, checked off on this branch).
- `ext-syntax transform` prints `file.sts(line,col): error <code>: message` (`8dba4d63e`).

#### Source maps

- The Sweetener map and the operator rewrite compose into one map, in the content-mapper hook and in `TransformSession::check`. A macro atom covers the lines the macro produced, and diagnostics in generated text point at the author file (`464c12ff6` via `0484c312f`; `TODO.md`).
- `rewrite_emitted_javascript` and the session emit path compose each `.js.map` through the rewrite's segments, so a generated position still names the author's token. Shifted columns are covered by a test (`0484c312f`).
- `__TYPEGPU_META__` is build output only and has no span map (`TODO.md`).

#### Diagnostics (SWR codes)

- Every Sweetener diagnostic uses one prefix (`8dba4d63e`). Reader errors are `SWR1002` (unexpected closing delimiter) and `SWR1003` (missing closing delimiter). A missing template directive, a syntax import that cannot be loaded, and "No rule for macro" are `SWR4001`.
- `expected.malformed.diagnostics.json` for the 16 malformed cases uses `SWR` only. `report_message` keeps a message that already starts with `SWR` or `TS`, and prints a bare code as `SWR<code>` (`8dba4d63e`).
- A failed expansion returns no virtual TypeScript, so the parser does not see the Sweetener source. A hole in `->` or `#` is not repaired into a call (`TODO.md`).

### Changed

#### Sweetener

- An operator-headed prefix form (`#[a, b]`) runs the compiled `(#)` rule. `macro_head` only saw identifiers, so the hardcoded `vector(...)` form won. `repaired_group_rejects` still refuses `#[1, , 3]` (`df2128f8c`).
- `take_token_tree` stops at a closed brace group, so `$body:tt` in `kinded Functor<F> { ... }` no longer swallows the next item (`f8d3c69aa`).
- A `#core` template is re-rendered by `expand_core`, and the hygiene plan is applied to the text that is printed. `#core` repetitions render per row through `expand_replacement`, so a repeated macro inside `module` still expands (`f8d3c69aa`, `3d5eb2a7f`).
- `render_rep` no longer rewrites a field path (`state.moves.event`) to its parent sequence. The plain longest own-path keeps the row scope of a nested repetition (`cdb320d13`).
- A data constructor's object body keeps the parentheses the macro template wrote, so `=> { tag: ... }` is an object literal and not a block (`96c44ce6b`).
- A standalone `$marker:token` hole takes one token. Only a repetition step (`$($rest:tt)+`) reads a token tree greedily. A field path walks to its nearest sequence ancestor (`cdd2dfec2`).
- Hygiene renaming finds a name nested in a template literal (`${globalThis.JSON.stringify(inspected)}`), not only `${inspected}` and `${inspected.` (`50c456805`).
- A macro name that is a parameter or a member stays out of macro dispatch. `is_operator_spelling` replaces the fixed single-character list, so a split `===` is consumed by an expression hole, and a literal right after an expression hole is that hole's stop (`339bbbbbe`).
- Each nested method segment binds its own parameter. A metavar hole no longer stamps every segment with the first parameter, and a finished call before an array comma is accepted (`a40856c4a`).
- A shadowed `function` the class rule does not take still matches, so the function-shadow case is rewritten before the fallback reprints it (`6c3d3dfb5`).
- A repeated hole no longer overwrites the same name outside the repetition. Operator literals span the reader's split `===` and `!==`, so the compiler fixture class matches (`351698e3a`).

#### TypeGPU

- `embed_metadata_reports` and the operator rewrite parse into this thread's detached store and drop it with the call. A transform no longer publishes into the process file registry, so two builds in one process do not share a parse (`a3b497903`).

### Fixed

#### Sweetener

Conformance against the previous HEAD, each with no new failure (`339bbbbbe` through `50c456805`):

- effect-do, effect-service, showcase, and source-aware-check: macro-call position guard and operator spelling (`339bbbbbe`, 17 to 13 failures).
- higher-kinded and units-of-measure: token-tree brace stop and hygiene after core render (`f8d3c69aa`, 17 to 11).
- state-machine: field-sequence parent walk dropped (`cdb320d13`, 11 to 10).
- workflow, new-language, and new-language/hygiene: `#core` repetitions per row (`3d5eb2a7f`, 10 to 9).
- vector-prefix: compiled rule for an operator-headed prefix (`df2128f8c`, 9 to 8).
- adt and adt/hygiene: parenthesized data-constructor object body (`96c44ce6b`, 8 to 6).
- typed-routes: a standalone token hole takes one token (`cdd2dfec2`, 6 to 5).
- `inspected` rename inside `dbg` and `check` template strings (`50c456805`, 5 failures against HEAD's 17).

### Removed

#### Sweetener

- Keyword rewrites for `query`, `table`, `memoized`, `service`, and the shadowed-function fallback, and the `useState`, `latestRef`, and `shadows core` source gates. Those cases expand through the imported macro (`8cd15b053`, `6c3d3dfb5`, `351698e3a`).
- Stale `diff.txt` captures once the expansion and `theirs` have the same shape (`6d2e1de14`, `8cd15b053`, `351698e3a`). `capture_ours` deletes `diff.txt` only in that case.
- `crates/ts_goport/examples/capture_typegpu.rs`, replaced by the TypeGPU tests (`a680a4ec8`).

### Known issues

- Sweetener build fix pending. `expand_apps` in `sweetener_apps.rs` still matches `error`, `effect name()`, `handle`, and `gen` by keyword. `match_runtime_import` still prefixes `import { c as _c } from "react/compiler-runtime"` when the file is the React compiler fixture and the macro module's import was not copied (`crates/ts_ext_syntax/TODO.md`).
- Three captured macro cases were still excluded when `cargo test -p ts_ext_sweetener --offline --lib --tests` was last recorded: `examples-react-memoization-src-function-shadow`, `fixtures-acceptance-playground-multi-part-methods`, and `fixtures-acceptance-playground-multi-part-methods-hygiene` (`NOTES-ext-finish.md`). Later commits (`6c3d3dfb5`, `a40856c4a`, `351698e3a`) retarget those three; `TODO.md` now marks them matching, and that note was not re-run.
- One API is open. `TransformSession` is not yet the only export, Sweetener and TypeGPU are not yet internal, and `session.check` / `session.transform` / `session.transformFile` with both passes defaulting on are unchecked (`crates/ts_ext_syntax/TODO.md`).
- Not started, from `NOTES-ext-finish.md`: `.sts` emit versus `ext/docs/sweetener.md`, the TypeGPU corpus as `#[test]`, `TransformSession` mappings feature bits, and the Vite build.

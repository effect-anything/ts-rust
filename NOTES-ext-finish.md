# ext-finish

Stopped after the expander step. Diagnostics, the single `TransformSession` API, LSP map tests, and `ext/examples/vite` are not started.

## Done

Sweetener expansion now runs the imported macro for signals, channels, and several playground forms instead of a library keyword table.

- `#metavar("hole", $name)` is a hole of the rule `#syntax` is defining. `#trim` prints the capture with surrounding space removed. `#index` is the current repetition row.
- A generated name such as `count` matches `count`, `count = 5`, `count += 1`, and `[count, flag]`. A closer (`(service)`) is not a call, so a parameter that shares a macro name is left alone.
- Relative runtime imports are rebased from the macro file onto the importing file. `for syntax` imports inside a macro module are loaded, so a template can call a macro it imported.
- Macros defined in the file being expanded are compiled. `export syntax` / `export operator` in that file are not printed as TypeScript.
- Template interpolations (`${label}`) expand when the name is a macro. That is how `effect { seen.push(`${label}`) }` becomes `labelSignal.get()`.
- Channel `->` / `<-` use the infix macro. The old rewrite that replaced the user's `put` / `take` declarations is gone.
- `doSteps` binders keep the author's spelling when an outer const has the same name (`left`, not `left_1`).
- `every_local_macro_case_matches_theirs` checks every captured case, including a macro that lives in `input.sts` rather than `macros.sts`.

`cargo test -p ts_ext_sweetener --offline --lib --tests` passed.

Deleted `diff.txt` for the cases that now match: CSP and CSP hygiene, React hooks, fine-jsx-implementation, currying hygiene, do-notation hygiene, new-language hygiene.

## Still open

These seven are named in the test and still differ. The suite does not fail on them.

- `examples-react-memoization-src-memoized`
- `examples-react-memoization-src-function-shadow`
- `examples-vite-react-src-main` (`deferred` does not match)
- `fixtures-acceptance-playground-adt` and `-hygiene`
- `fixtures-acceptance-playground-multi-part-methods` and `-hygiene`

The form pass still prints service, query, table, memoized, and the React compiler fixture when the compiled rule does not match. `useState` / `latestRef` / signal `state` are no longer keyword rewrites.

## Not started

- One diagnostic prefix (`error TS<code>:`). Reader and macro errors are still `SWR1002`, `SWR1003`, and `SWR4001`.
- `TransformSession({ typegpu, sweetener })` with `check` / `transform` / `transformFile`. The old `check()` / `build()` constructors are still the API.
- LSP source-map edge tests.
- `ext/examples/vite` build, and the extra Effect / TypeGPU examples.

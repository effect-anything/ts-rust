# ext-finish

Stopped after the expander step, on purpose. P0 review items, TypeGPU corpus, mappings feature bits, diagnostic codes, `TransformSession`, LSP tests, and the Vite example were not started. Those go to other workers.

## Done

Commit `6d2e1de14` plus the follow-up on this branch.

- `#metavar` holes are suffixed with the binding they belong to (`argument_minimum`), so two parameters do not share one capture. Nested segment repetitions still stamp every segment with the first parameter, so multi-part methods still drop `upper(5)`.
- JSX text such as `deferred {deferredCount}` is not a statement macro. A failed rule there no longer discards the file.
- Names produced by `#join` are the author's binding. A second function may reuse `toggleDetailsOpen`. A template parameter such as `current` still becomes `current_1`.
- Repetition rows keep every hole from the same iteration, so `states { page = 1; query = ""; }` prints both names.
- `data` / constructor `match` no longer rewrite `<T,>` away, invent `void None`, or rename a constructor field that shadows an outer `const`. The empty `None` arm keeps its braces.
- `service` is not a keyword rewrite anymore. The imported macro matches those cases.
- `query` and `table` are still form-pass fallbacks. Removing them loses `examples-library-macros-drizzle-schema-main` (shape) and `examples-library-macros-drizzle-query-main` (`TS4001`).
- The React compiler fixture form still prints `memoized`. The syntax class does not accept that JSX (it stops on `/`), so removing the form loses the case. `function` with `shadows core` still does not match `examples-react-memoization-src-function-shadow`.

`cargo test -p ts_ext_sweetener --offline --lib --tests` passed with the three names below excluded.

Shape-matched and `diff.txt` removed since `6d2e1de14`: vite-react main, both playground adt cases, memoized. Earlier commit already removed CSP, React hooks, fine-jsx-implementation, currying hygiene, do-notation hygiene, and new-language hygiene.

## Still open (listed on the shape test)

- `examples-react-memoization-src-function-shadow`
- `fixtures-acceptance-playground-multi-part-methods`
- `fixtures-acceptance-playground-multi-part-methods-hygiene`

## Not started

- JS source map after `rewrite_emitted_javascript` / API `.map` rewrite (P0).
- `.sts` emit versus `ext/docs/sweetener.md` (P0).
- Global parse store in `embed_metadata_reports` (P0).
- TypeGPU corpus as `#[test]` (P1).
- `TransformSession` mappings feature bits (P1).
- One diagnostic prefix, One API, LSP/content-mapper tests, Vite build and extra examples.

# Extensions follow-up

Sweetener cases are flat directories under `crates/ts_ext_sweetener/cases`, one program per `cases.jsonl` row. `vendor/` is a gitignored pair of symlinks to local reference checkouts. The port does not read those trees. Discover and sync may.

## Diagnostics

- [x] A failed expansion returns no virtual TypeScript, so the parser does not see the Sweetener source
- [x] Other files stay separate transforms
- [x] The 16 `malformed` cases match the phrase in `expected.malformed.diagnostics.json`. A hole in `->` or `#` is not repaired into a call
- [x] `ext-syntax transform` prints `file.sts(line,col): error TSxxxx: message`

## Source map

- [x] Compose the Sweetener map and the operator rewrite into one map, in the content-mapper hook and in `TransformSession::check`
- [ ] Framework lowering (`__TYPEGPU_META__`) is still not a span map. Build output does not point back through that wrapper
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

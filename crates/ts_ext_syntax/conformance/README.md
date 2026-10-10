# Conformance snapshots

Each case is one directory. The input stays a symlink into `vendor/`. The
two outputs are real files. A drift on either side shows up as a diff of
those files.

```text
conformance/cases/<id>/
  input.*     symlink into vendor/ (their source, not a copy)
  theirs.*    output of their program, written by us
  ours.*      output of ts_ext_syntax::expand
  diff.txt    optional; only when theirs and ours differ
```

Do not copy their source into the case. A copied input can be edited by
accident and then both outputs match the wrong program. The symlink fails
if their checkout moves, which is the failure we want.

`theirs` and `ours` are copies of program output, not links. Their repos do
not store our output, and we need both strings in this tree so a later sync
of either program is a visible diff.

## What is frozen

| file | who writes it | when it changes |
| --- | --- | --- |
| `input` | symlink | never, except the link target |
| `theirs` | their program, via the capture step | their repo moved, or we re-capture |
| `ours` | `expand` | our pass changed |

Check and build are the same pair, not new oracles:

- `theirs.check.txt` / `ours.check.txt`: diagnostic text from their checker
  and from ours, on that case's expanded text.
- `theirs.build.txt` / `ours.build.txt`: what their bundler plugin emits,
  and what `ext_syntax_build::transform` emits.

Expand is the first pair (`theirs.ts` / `ours.ts`). Do not add a third copy
of the same string under another name.

## Capture order

1. Create the case directory and the input symlink.
2. Run their program on that input. Write stdout to `theirs.ts`. If their
   program cannot run, write `theirs.ts.missing` with the reason and stop
   that case. Do not invent their output.
3. Run `expand` on the same input bytes. Write `ours.ts`.
4. Diff `theirs.ts` and `ours.ts`. Equal bytes: delete `diff.txt` if it
   exists. Unequal: write a unified diff to `diff.txt`. A missing `theirs`
   is not a pass.

Re-capture is explicit. A normal test run does not call their program and
does not overwrite `theirs.ts`. It re-runs `expand`, rewrites `ours.ts` only
in a capture mode, and in check mode fails if `ours.ts` would change or if
`ours.ts` != `theirs.ts`.

That split is what catches sync drift:

- Their repo moved and we re-capture: `theirs.ts` changes, `ours.ts` does
  not, `diff.txt` appears.
- Our pass moved: `ours.ts` changes, `theirs.ts` does not, `diff.txt`
  appears.
- Both moved the same way: both files change and the diff is empty. The
  commit still shows both snapshots moving, so the sync is visible.

## TypeGPU

Their tests keep the expected rewrite inside
`use-gpu-directive.test.ts` as a snapshot. The capture step runs their
babel transform (`test/transform.ts` `babelTransform`) on the input and
writes that string to `theirs.ts`. It does not scrape the snapshot as the
oracle. The snapshot is their regression for themselves. `theirs.ts` is our
copy of a live run, so we match the program, not a stale snapshot.

Strip nothing unless the manifest for that case says `strip: typegpu-meta`.
Operator-only cases set that, because `__TYPEGPU_META__` is their build
post-step and is compared in `theirs.build.txt`, not in the expand pair.

## Sweetener

`expected.ts` in their fixture is a checked-in oracle, not a live run. The
capture step still runs their compiler (`session.transform`) and writes
`theirs.ts`. After a successful run, also diff `theirs.ts` against their
`expected.ts`. If those two differ, their fixture and their program have
diverged; record that in `theirs.ts.note` and fail the case. We match the
program. The note says the fixture is stale.

## Manifest

`cases.jsonl` lists admitted cases. `status: pending` means the directory
does not exist yet. `status: captured` means `input`, `theirs.ts`, and
`ours.ts` exist. A listed case with no directory fails the conformance
test. A directory with no manifest line fails too.

Ids use the path under `cases/`, for example
`sweetener/playground/operators`.

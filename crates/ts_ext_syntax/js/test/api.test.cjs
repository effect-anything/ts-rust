// node --test js/test: the Node API contract (needs the addon; see README "Build the addon").
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const { Transformer, formatDiagnostics } = require("../index.cjs");

const t = new Transformer({ tsconfig: false });
const macro = "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n";

test("errors and warnings split diagnostics", () => {
  const r = t.transform(`${macro}export const a = twice(1);\nunknown syntax here (;\n`, "/p/a.sts");
  assert.deepEqual(
    [...r.errors, ...r.warnings].sort((a, b) => a.start - b.start),
    [...r.diagnostics].sort((a, b) => a.start - b.start),
  );
  assert.ok(r.errors.every((d) => d.category === "error"));
  assert.ok(r.warnings.every((d) => d.category !== "error"));
  assert.equal(typeof formatDiagnostics(r.warnings), "string");
});

test("an unchanged file has the identity map and composes through it", () => {
  const code = "export const a = 1;\nexport const b = 2;\n";
  const r = t.transform(code, "/p/plain.ts");
  assert.equal(r.changed, false);
  assert.equal(r.map.version, 3);
  assert.deepEqual(r.map.sources, ["/p/plain.ts"]);
  const later = { version: 3, sources: ["plain.ts"], names: [], mappings: "AAAA;AACA" };
  const composed = r.composeMap(later);
  assert.deepEqual(composed.sources, ["/p/plain.ts"]);
  assert.equal(composed.mappings, "AAAA;AACA");
});

test(".sjs and .sjsx are refused", () => {
  for (const path of ["/p/a.sjs", "/p/a.sjsx"]) {
    assert.throws(() => t.transform("export const a = 1;", path), /not supported/);
  }
});

// Build the example, then decode the bundle's .js.map and check that
// tokens produced by Sweetener macros point back at their `.sts` text.
import assert from "node:assert/strict";
import { readFileSync, readdirSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";
import { build } from "vite";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const VLQ = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function decode(mappings) {
  const out = [];
  let [source, sourceLine, sourceColumn] = [0, 0, 0];
  mappings.split(";").forEach((text, line) => {
    let column = 0;
    for (const segment of text.split(",")) {
      if (!segment) continue;
      const fields = [];
      let [value, shift] = [0, 0];
      for (const ch of segment) {
        const digit = VLQ.indexOf(ch);
        value += (digit & 31) * 2 ** shift;
        shift += 5;
        if (!(digit & 32)) {
          fields.push(value & 1 ? -Math.floor(value / 2) : Math.floor(value / 2));
          [value, shift] = [0, 0];
        }
      }
      column += fields[0];
      if (fields.length < 4) continue;
      source += fields[1];
      sourceLine += fields[2];
      sourceColumn += fields[3];
      out.push({ line, column, source, sourceLine, sourceColumn });
    }
  });
  return out;
}

test("bundle .js.map maps macro output back to .sts tokens", async () => {
  await build({ root, logLevel: "silent", configFile: join(root, "vite.config.js") });
  const assets = join(root, "dist/assets");
  const js = readdirSync(assets).find((f) => f.endsWith(".js"));
  const code = readFileSync(join(assets, js), "utf8");
  const map = JSON.parse(readFileSync(join(assets, `${js}.map`), "utf8"));
  const points = decode(map.mappings);
  const original = (needle, delta = 0) => {
    const found = code.indexOf(needle);
    assert.ok(found >= 0, `bundle lacks ${needle}`);
    const at = found + delta;
    const before = code.slice(0, at).split("\n");
    const line = before.length - 1;
    const column = before[line].length;
    const hit = points.find((p) => p.line === line && p.column === column);
    assert.ok(hit, `no mapping at ${needle}`);
    const name = map.sources[hit.source];
    const text = map.sourcesContent[hit.source].split("\n")[hit.sourceLine];
    return { name, text: text.slice(hit.sourceColumn) };
  };
  // `petal.millimeters |> formatMillimeters` expands to a call; each piece
  // keeps its own author position.
  // Rolldown inlines `const petal = new Length(petalMm)` into the call.
  const call = "formatMillimeters(new Length(petalMm).millimeters)";
  const fn = original(call);
  assert.match(fn.name, /flower\.sts$/);
  assert.ok(fn.text.startsWith("formatMillimeters;"), fn.text);
  const arg = original(call, call.indexOf("millimeters"));
  assert.ok(arg.text.startsWith("millimeters |>"), arg.text);
  // `unless (speed > 0)` keeps the condition's author column.
  const cond = original("speed > 0");
  assert.ok(cond.text.startsWith("speed > 0) {"), cond.text);
  // A tsover operator call maps to its operator expression in flower.sts.
  const op = code.match(/__tsover_\w+\(/);
  assert.ok(op, "bundle has no tsover call");
  const gpu = original(op[0]);
  assert.match(gpu.name, /flower\.sts$/);
});

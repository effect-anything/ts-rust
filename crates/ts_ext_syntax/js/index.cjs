// The @effect-x/tsc Node API. See index.d.ts for the contract.
//
// The class name is one binding (`Transformer` below); rename it there and in
// index.d.ts. The native side (src/node.rs) answers in JSON and knows no
// names.
"use strict";
const fs = require("node:fs");

let native;
function binding() {
  native ??= require("./native.cjs");
  return native;
}

const CATEGORIES = ["sweetener", "typegpu"];

/** Per-category merge: fields of `over` replace those of `base`. */
function mergeOptions(base, over) {
  const out = {};
  for (const category of CATEGORIES) {
    const merged = { ...(base?.[category] ?? {}), ...(over?.[category] ?? {}) };
    for (const key of Object.keys(merged)) if (merged[key] === undefined) delete merged[key];
    out[category] = merged;
  }
  for (const source of [base, over]) {
    for (const key of Object.keys(source ?? {})) {
      if (key !== "tsconfig" && !CATEGORIES.includes(key)) {
        throw new TypeError(`Transformer: unknown option '${key}'`);
      }
    }
  }
  return out;
}

class Transformer {
  #options;
  #tsconfig;

  constructor(options = {}) {
    this.#tsconfig = options.tsconfig;
    this.#options = mergeOptions(undefined, options);
  }

  transform(code, path, options = {}) {
    if (typeof code !== "string") throw new TypeError("transform(code, path): code must be a string");
    if (typeof path !== "string") throw new TypeError("transform(code, path): path must be a string");
    const tsconfig = options.tsconfig !== undefined ? options.tsconfig : this.#tsconfig;
    const merged = mergeOptions(this.#options, options);
    const raw = JSON.parse(
      binding().transformNative(
        code,
        path,
        typeof tsconfig === "string" ? tsconfig : undefined,
        tsconfig !== false,
        JSON.stringify(merged),
      ),
    );
    return makeResult(raw, code, path);
  }

  transformFile(path, options) {
    return this.transform(fs.readFileSync(path, "utf8"), path, options);
  }
}

function makeResult(raw, source, path) {
  const { spans, ...fields } = raw;
  return {
    code: fields.code,
    hostPath: fields.hostPath,
    dependencies: fields.dependencies,
    diagnostics: fields.diagnostics,
    errors: fields.diagnostics.filter((item) => item.category === "error"),
    warnings: fields.diagnostics.filter((item) => item.category !== "error"),
    map: fields.map,
    changed: fields.changed,
    tsconfig: fields.tsconfig,
    composeMap(map) {
      if (map == null) throw new TypeError("composeMap(map): map must be a source map");
      const text = typeof map === "string" ? map : JSON.stringify(map);
      return JSON.parse(binding().composeNative(path, fields.hostPath, source, fields.code, spans, text));
    },
  };
}

/** `file(line,column): category SWRcode: message`, related locations indented. */
function formatDiagnostics(diagnostics) {
  const lines = [];
  for (const item of diagnostics) {
    const code = item.code ? `SWR${item.code}: ` : "";
    lines.push(`${item.file}(${item.line},${item.column}): ${item.category} ${code}${item.message}`);
    for (const related of item.related ?? []) {
      lines.push(`  ${related.file}(${related.line},${related.column}): ${related.message}`);
    }
  }
  return lines.join("\n");
}

exports.Transformer = Transformer;
exports.formatDiagnostics = formatDiagnostics;

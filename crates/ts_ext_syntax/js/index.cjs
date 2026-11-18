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

/** Explicit options as JSON for the native side, or undefined. */
function projectOptions(options) {
  const { tsconfig, ...rest } = options ?? {};
  for (const key of Object.keys(rest)) {
    if (!CATEGORIES.includes(key)) throw new TypeError(`Project: unknown option '${key}'`);
  }
  return Object.keys(rest).length ? JSON.stringify(mergeOptions(undefined, rest)) : undefined;
}

function fileResult(raw, path) {
  const { spans, ...fields } = raw;
  return {
    path,
    code: fields.code,
    map: fields.map,
    moduleType: fields.moduleType,
    hostPath: fields.hostPath,
    changed: fields.changed,
    dependencies: fields.dependencies,
    diagnostics: fields.diagnostics,
    errors: fields.diagnostics.filter((item) => item.category === "error"),
    warnings: fields.diagnostics.filter((item) => item.category !== "error"),
  };
}

class Project {
  #id;
  #info;

  constructor(id, info) {
    this.#id = id;
    this.#info = info;
  }

  static open(options) {
    if (typeof options?.tsconfig !== "string") throw new TypeError("Project.open: tsconfig must be a path");
    const info = JSON.parse(binding().projectOpen(options.tsconfig, projectOptions(options)));
    return new Project(info.id, info);
  }

  get tsconfig() {
    return this.#info.tsconfig;
  }
  get extensions() {
    return this.#info.extensions;
  }
  get configErrors() {
    return this.#info.configErrors;
  }

  #handle() {
    if (this.#id === undefined) throw new Error("Project is disposed");
    return this.#id;
  }

  owns(path) {
    return binding().projectOwns(this.#handle(), path);
  }

  files() {
    return binding().projectFiles(this.#handle());
  }

  getFile(path, text) {
    return fileResult(JSON.parse(binding().projectGetFile(this.#handle(), path, text)), path);
  }

  async getFileAsync(path, text) {
    return fileResult(JSON.parse(await binding().projectGetFileAsync(this.#handle(), path, text)), path);
  }

  invalidate(path) {
    return binding().projectInvalidate(this.#handle(), path);
  }

  dispose() {
    if (this.#id === undefined) return;
    binding().projectDispose(this.#id);
    this.#id = undefined;
  }

  [Symbol.dispose]() {
    this.dispose();
  }
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
exports.Project = Project;
exports.formatDiagnostics = formatDiagnostics;

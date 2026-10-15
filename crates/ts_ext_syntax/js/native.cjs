// Loads the tsc-ext Node addon.
//
// Published: lib/tsc-ext.node in the platform package <name>-<platform>-<arch>
// (for example @effect-x/tsc-darwin-arm64), next to the tsc and tsc-ext
// binaries. Development (crates/ts_ext_syntax): the `napi build --platform`
// output tsc-ext.<platform>-<arch>[-<abi>].node in the crate directory.
// TSC_EXT_NODE=<path> overrides both.
"use strict";
const fs = require("node:fs");
const path = require("node:path");

function candidates() {
  const out = [];
  if (process.env.TSC_EXT_NODE) out.push(process.env.TSC_EXT_NODE);
  const triple = `${process.platform}-${process.arch}`;
  let name = "@effect-x/tsc";
  for (const dir of [path.join(__dirname, ".."), path.join(__dirname, "..", "..")]) {
    try {
      name = JSON.parse(fs.readFileSync(path.join(dir, "package.json"), "utf8")).name || name;
      break;
    } catch {}
  }
  try {
    const pkg = require.resolve(`${name}-${triple}/package.json`, { paths: [__dirname] });
    out.push(path.join(path.dirname(pkg), "lib", "tsc-ext.node"));
  } catch {}
  const crate = path.join(__dirname, "..");
  for (const suffix of ["", "-gnu", "-musl", "-msvc"]) {
    out.push(path.join(crate, `tsc-ext.${triple}${suffix}.node`));
  }
  return out;
}

function load() {
  const tried = [];
  for (const file of candidates()) {
    if (!fs.existsSync(file)) {
      tried.push(file);
      continue;
    }
    return require(file);
  }
  throw new Error(
    `@effect-x/tsc: no tsc-ext Node addon for ${process.platform}-${process.arch}. Looked at:\n  ${tried.join("\n  ")}`,
  );
}

module.exports = load();

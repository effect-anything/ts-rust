import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import { mkdirSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { TransformSession } from "ext-syntax";

const here = dirname(fileURLToPath(import.meta.url));
const example = process.cwd();
const src = join(example, "src");
const out = join(example, "generated");

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

const session = TransformSession.build();
const files = [];
let javascript = "";
let jsx = false;

for (const name of readdirSync(src)) {
  if (name.endsWith(".d.ts")) {
    writeFileSync(join(out, name), readFileSync(join(src, name)));
    files.push(name);
    continue;
  }
  if (!/\.(ts|tsx|sts|stsx)$/.test(name)) continue;
  const sourcePath = join(src, name);
  const transformed = session.transform(sourcePath, readFileSync(sourcePath, "utf8"));
  if (transformed.diagnostics.length > 0) {
    for (const item of transformed.diagnostics) {
      console.error(`${name}: ${item.message}`);
    }
    process.exit(1);
  }
  const outName = name.replace(/\.stsx$/, ".tsx").replace(/\.sts$/, ".ts");
  writeFileSync(join(out, outName), transformed.code);
  writeFileSync(join(out, `${outName}.map`), transformed.sourceMap);
  files.push(outName);
  javascript += transformed.code;
  jsx ||= outName.endsWith(".tsx");
}

const needsTypegpu = javascript.includes('from "typegpu"') || javascript.includes("from 'typegpu'");
if (needsTypegpu) {
  writeFileSync(join(out, "typegpu.d.ts"), readFileSync(join(here, "stubs/typegpu.d.ts")));
  files.push("typegpu.d.ts");
}
if (jsx) {
  writeFileSync(join(out, "jsx.d.ts"), readFileSync(join(here, "stubs/jsx.d.ts")));
  files.push("jsx.d.ts");
}
writeFileSync(join(out, "runtime.d.ts"), readFileSync(join(here, "runtime.d.ts")));
files.push("runtime.d.ts");

const compilerOptions = {
  strict: true,
  target: "esnext",
  module: "esnext",
  moduleResolution: "bundler",
  rootDir: ".",
  outDir: "../dist",
  skipLibCheck: true,
};
if (javascript.includes("__TYPEGPU_META__")) compilerOptions.noImplicitAny = false;
if (jsx) compilerOptions.jsx = "react-jsx";

writeFileSync(
  join(out, "tsconfig.json"),
  JSON.stringify({ compilerOptions, files }, null, 2),
);

const require = createRequire(import.meta.url);
const tsc = require.resolve("typescript/bin/tsc");
const result = spawnSync(tsc, ["-p", join(out, "tsconfig.json")], {
  stdio: "inherit",
});
if (result.error) {
  console.error(result.error.message);
  process.exit(1);
}
process.exit(result.status ?? 1);

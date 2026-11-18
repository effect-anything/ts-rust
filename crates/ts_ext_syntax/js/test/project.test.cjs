// node --test js/test: the Project API (needs the addon).
"use strict";
const test = require("node:test");
const assert = require("node:assert/strict");
const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { Project } = require("../index.cjs");

function fixture() {
  const dir = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsc-ext-project-")));
  fs.mkdirSync(path.join(dir, "src"));
  fs.writeFileSync(
    path.join(dir, "tsconfig.json"),
    JSON.stringify({
      compilerOptions: { strict: true, jsx: "preserve" },
      contentMappers: [{ package: "@effect-x/tsc", extensions: [".sts", ".stsx"] }],
      include: ["src"],
    }),
  );
  fs.writeFileSync(path.join(dir, "src/macros.sts"), "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n");
  fs.writeFileSync(path.join(dir, "src/main.sts"), 'import { twice } from "./macros.sts" for syntax;\nexport const a = twice(1);\n');
  fs.writeFileSync(path.join(dir, "src/view.stsx"), "export const v = <div />;\n");
  fs.writeFileSync(path.join(dir, "src/plain.ts"), "export const p = 1;\n");
  fs.writeFileSync(path.join(dir, "src/types.d.ts"), "export {};\n");
  return dir;
}

test("Project owns, serves and invalidates", async () => {
  const dir = fixture();
  const project = Project.open({ tsconfig: path.join(dir, "tsconfig.json") });
  try {
    const main = path.join(dir, "src/main.sts");
    assert.ok(project.owns(main), JSON.stringify(project.files()));
    assert.ok(project.owns(path.join(dir, "src/plain.ts")));
    assert.ok(!project.owns(path.join(dir, "src/types.d.ts")));
    assert.ok(!project.owns(path.join(dir, "elsewhere.ts")));
    const r = project.getFile(main);
    assert.equal(r.moduleType, "ts");
    assert.equal(r.map.version, 3);
    assert.deepEqual(r.errors, []);
    assert.equal(project.getFile(path.join(dir, "src/view.stsx")).moduleType, "tsx");
    const a = await project.getFileAsync(main);
    assert.equal(a.code, r.code);
    const affected = project.invalidate(path.join(dir, "src/macros.sts"));
    assert.ok(affected.includes(main), JSON.stringify(affected));
    const all = project.invalidate(path.join(dir, "tsconfig.json"));
    assert.ok(all.length >= 3);
  } finally {
    project.dispose();
  }
  assert.throws(() => project.owns("x"), /disposed/);
});

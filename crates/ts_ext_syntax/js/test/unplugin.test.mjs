// The Vite plugin's load and HMR invalidation, driven without a server.
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import test from "node:test";
import vite from "../unplugin/vite.mjs";

const call = (hook, ctx, ...args) => (typeof hook === "function" ? hook : hook.handler).apply(ctx, args);

test("load serves owned files; hotUpdate invalidates dependents", async () => {
  const root = fs.realpathSync(fs.mkdtempSync(path.join(os.tmpdir(), "tsc-ext-unplugin-")));
  fs.mkdirSync(path.join(root, "src"));
  fs.writeFileSync(
    path.join(root, "tsconfig.json"),
    JSON.stringify({ contentMappers: [{ package: "@effect-x/tsc", extensions: [".sts", ".stsx"] }], include: ["src"] }),
  );
  const macros = path.join(root, "src/macros.sts");
  const main = path.join(root, "src/main.sts");
  fs.writeFileSync(macros, "export syntax twice:expr {\n  rule { twice($x:expr) } => { [$x, $x] }\n}\n");
  fs.writeFileSync(main, 'import { twice } from "./macros.sts" for syntax;\nexport const a = twice(1);\n');

  const plugin = vite();
  call(plugin.configResolved, plugin, { root });
  const watched = [];
  const ctx = { warn() {}, error(e) { throw new Error(e.message); }, addWatchFile: (f) => watched.push(f) };
  call(plugin.buildStart, ctx);
  assert.equal(await call(plugin.resolveId, ctx, "./main", path.join(root, "src/x.ts")), main);
  const loaded = await call(plugin.load, ctx, main);
  assert.ok(loaded, "main.sts is owned");
  assert.equal(loaded.moduleType, "ts");
  assert.match(loaded.code, /\[1, 1\]/);
  assert.equal(loaded.map.version, 3);
  assert.ok(watched.includes(macros), JSON.stringify(watched));
  assert.equal(await call(plugin.load, ctx, path.join(root, "outside.ts")), undefined);

  const invalidated = [];
  const mainModule = { file: main };
  const environment = {
    moduleGraph: {
      getModulesByFile: (file) => (file === main ? new Set([mainModule]) : undefined),
      invalidateModule: (mod) => invalidated.push(mod),
    },
  };
  const out = call(plugin.hotUpdate, { environment }, { file: macros, modules: [] });
  assert.deepEqual(invalidated, [mainModule]);
  assert.ok(out.includes(mainModule));
});

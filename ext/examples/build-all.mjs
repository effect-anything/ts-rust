import { spawnSync } from "node:child_process";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(fileURLToPath(import.meta.url));
const names = ["sweetener", "tsover", "typegpu", "vite"];

for (const name of names) {
  console.log(`\n# ${name}`);
  const result = spawnSync("npm", ["run", "build"], {
    cwd: join(root, name),
    stdio: "inherit",
  });
  if ((result.status ?? 1) !== 0) process.exit(result.status ?? 1);
}

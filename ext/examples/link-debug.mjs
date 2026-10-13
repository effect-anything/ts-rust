import { existsSync, mkdirSync, rmSync, symlinkSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const examplesDir = dirname(fileURLToPath(import.meta.url));
const repoRoot = join(examplesDir, "..", "..");

linkBinary(
  join(repoRoot, "target", "debug", "tsgo"),
  join(examplesDir, "..", "typescript", "bin", "tsc"),
  join("..", "..", "..", "target", "debug", "tsgo"),
  "cargo build -p ts_goport --bin tsgo",
);
linkBinary(
  join(repoRoot, "target", "debug", "tsc-ext"),
  join(repoRoot, "crates", "ts_ext_syntax", "bin", "tsc-ext"),
  join("..", "..", "..", "target", "debug", "tsc-ext"),
  "cargo build -p ts_ext_syntax --bin tsc-ext",
);

function linkBinary(debug, dest, relative, build) {
  if (!existsSync(debug)) {
    console.error(`examples: no debug binary at ${debug}`);
    console.error(`build it with: ${build}`);
    process.exit(1);
  }
  mkdirSync(dirname(dest), { recursive: true });
  rmSync(dest, { force: true });
  symlinkSync(relative, dest);
}

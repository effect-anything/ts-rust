import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { Transformer, formatDiagnostics } from "@effect-x/tsc/ext";
import { transformWithEsbuild } from "vite";

// Options come from tsconfig.json (contentMappers "@effect-x/tsc" options).
const transformer = new Transformer();

function strip(id) {
  return id.split("?")[0];
}

function publish(ctx, file, code) {
  const result = transformer.transform(code, file);
  for (const dependency of result.dependencies) ctx.addWatchFile(dependency);
  for (const item of result.warnings) {
    ctx.warn({ message: formatDiagnostics([item]), id: file, pos: item.start });
  }
  if (result.errors.length > 0) {
    const [first] = result.errors;
    ctx.error({ message: formatDiagnostics(result.errors), id: file, pos: first.start });
  }
  if (!result.changed) return null;
  // `map` is a Source Map v3 with exact per-token mappings back to `file`.
  return { code: result.code, map: result.map };
}

export function tscExt() {
  return {
    name: "tsc-ext",
    enforce: "pre",
    async resolveId(source, importer) {
      if (!importer || !source.startsWith(".")) return null;
      const target = resolve(dirname(strip(importer)), source.split("?")[0]);
      if (target.endsWith(".sts") || target.endsWith(".stsx")) {
        return existsSync(target) ? target : null;
      }
      if (
        existsSync(`${target}.ts`) ||
        existsSync(`${target}.tsx`) ||
        existsSync(`${target}.js`) ||
        existsSync(`${target}.jsx`)
      ) {
        return null;
      }
      if (existsSync(`${target}.stsx`)) return `${target}.stsx`;
      if (existsSync(`${target}.sts`)) return `${target}.sts`;
      return null;
    },
    transformIndexHtml(html) {
      return html.replaceAll(
        'src="/src/main.sts"',
        'src="/src/main.sts?import"',
      );
    },
    async transform(code, id) {
      const file = strip(id);
      if (file.includes("node_modules")) return null;
      const sweet = /\.stsx?$/.test(file);
      if (!sweet && !/\.(tsx|ts)$/.test(file)) return null;
      const published = publish(this, file, code);
      if (!sweet) return published;
      // An unchanged `.sts` file is still source. Esbuild has to emit it or
      // the browser receives the raw file with no JavaScript content type.
      const source = published ?? { code, map: null };
      const result = await transformWithEsbuild(
        source.code,
        file,
        {
          loader: file.endsWith(".stsx") ? "tsx" : "ts",
          jsx: "automatic",
          sourcemap: true,
        },
        source.map,
      );
      return { code: result.code, map: result.map };
    },
  };
}

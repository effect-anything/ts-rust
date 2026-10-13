import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { TransformSession } from "@effect-x/tsc";
import { transformWithEsbuild } from "vite";

const session = TransformSession.build();

function strip(id) {
  return id.split("?")[0];
}

function publish(ctx, file, code) {
  const result = session.transform(file, code);
  for (const item of result.diagnostics) {
    const related = (item.related ?? []).map(
      (at) => `\n  ${file}:${where(code, at.start)} ${at.message}`,
    );
    const message = `${file}:${where(code, item.start)} ${item.message}${related.join("")}`;
    if (item.severity === "warning") {
      ctx.warn({ message, id: file, pos: item.start });
      continue;
    }
    ctx.error({ message, id: file, pos: item.start });
  }
  if (result.code === code) return null;
  return { code: result.code, map: sourceMap(file, code, result) };
}

function where(code, offset) {
  const before = Buffer.from(code).subarray(0, offset).toString();
  const lines = before.split("\n");
  return `${lines.length}:${lines[lines.length - 1].length + 1}`;
}

// The session returns a Source Map v3 with exact per-token `mappings`
// (UTF-16 columns, `sourcesContent`). Hand it to the bundler as is; the
// raw spanmap `segments` are only for tools that compose span maps.
function sourceMap(file, original, result) {
  let map;
  try {
    map = JSON.parse(result.sourceMap);
  } catch {
    return null;
  }
  if (typeof map.mappings !== "string") return null;
  return {
    version: 3,
    file,
    sources: [file],
    sourcesContent: map.sourcesContent ?? [original],
    names: map.names ?? [],
    mappings: map.mappings,
  };
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

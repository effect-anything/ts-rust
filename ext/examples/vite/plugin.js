import { existsSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { TransformSession } from "ext-syntax";
import { transformWithEsbuild } from "vite";

const session = TransformSession.build();

function strip(id) {
  return id.split("?")[0];
}

function publish(ctx, file, code) {
  const result = session.transform(file, code);
  for (const item of result.diagnostics) {
    // SWR4025 is a warning upstream: the item is passed through as written.
    if (item.message.startsWith("SWR4025")) {
      ctx.warn({ message: `${file}: ${item.message}`, id: file });
      continue;
    }
    ctx.error({ message: `${file}: ${item.message}`, id: file });
  }
  if (result.code === code) return null;
  return { code: result.code, map: sourceMap(file, code, result) };
}

const VLQ = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

function encodeVLQ(value) {
  let n = value < 0 ? (-value << 1) | 1 : value << 1;
  let out = "";
  do {
    let digit = n & 31;
    n >>>= 5;
    if (n > 0) digit |= 32;
    out += VLQ[digit];
  } while (n > 0);
  return out;
}

function indexAtByte(text, byte) {
  const bytes = Buffer.from(text, "utf8");
  return bytes.subarray(0, Math.min(byte, bytes.length)).toString("utf8").length;
}

function lineCol(text, index) {
  let line = 0;
  let column = 0;
  const end = Math.min(index, text.length);
  for (let i = 0; i < end; i++) {
    if (text.charCodeAt(i) === 10) {
      line += 1;
      column = 0;
    } else {
      column += 1;
    }
  }
  return [line, column];
}

function sourceMap(file, original, result) {
  let segments = [];
  try {
    segments = JSON.parse(result.sourceMap).segments ?? [];
  } catch {
    return null;
  }
  const points = [];
  for (const [start, length, originalStart, originalLength, kind] of segments) {
    if (length <= 0) continue;
    const genFrom = indexAtByte(result.code, start);
    const genTo = indexAtByte(result.code, start + length);
    const origFrom = indexAtByte(original, originalStart);
    const sameBytes =
      Buffer.from(result.code, "utf8")
        .subarray(start, start + length)
        .toString("utf8") ===
      Buffer.from(original, "utf8")
        .subarray(originalStart, originalStart + originalLength)
        .toString("utf8");
    const verbatim = kind === 0 && originalLength === length && sameBytes;
    if (kind === 0 && !verbatim) continue;
    let genIndex = genFrom;
    let origIndex = origFrom;
    let first = true;
    while (genIndex < genTo) {
      if (first || result.code.charCodeAt(genIndex - 1) === 10) {
        points.push([...lineCol(result.code, genIndex), ...lineCol(original, origIndex)]);
        first = false;
      }
      genIndex += 1;
      if (verbatim) origIndex += 1;
    }
  }
  points.sort((a, b) => a[0] - b[0] || a[1] - b[1]);
  const lines = [];
  let line = 0;
  let column = 0;
  let sourceLine = 0;
  let sourceColumn = 0;
  for (const [nextLine, nextColumn, nextSourceLine, nextSourceColumn] of points) {
    while (lines.length <= nextLine) lines.push([]);
    const sameLine = lines[nextLine].length > 0;
    lines[nextLine].push(
      encodeVLQ(nextColumn - (sameLine ? column : 0)) +
        encodeVLQ(0) +
        encodeVLQ(nextSourceLine - sourceLine) +
        encodeVLQ(nextSourceColumn - sourceColumn),
    );
    line = nextLine;
    column = nextColumn;
    sourceLine = nextSourceLine;
    sourceColumn = nextSourceColumn;
  }
  return {
    version: 3,
    file,
    sources: [file],
    sourcesContent: [original],
    names: [],
    mappings: lines.map((groups) => groups.join(",")).join(";"),
  };
}

export function extSyntax() {
  return {
    name: "ext-syntax",
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

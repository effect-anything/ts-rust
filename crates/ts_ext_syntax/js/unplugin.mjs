// The @effect-x/tsc bundler plugin ("@effect-x/tsc/unplugin"): the project's
// own files (tsconfig include/files, without node_modules and declarations)
// are loaded through `Project`, so the bundler sees the rewritten TS/TSX with
// an exact map back to the source. See unplugin.d.mts.
import { existsSync } from "node:fs";
import { createRequire } from "node:module";
import path from "node:path";
import { pathToFileURL } from "node:url";
import { createUnplugin } from "unplugin";

const require = createRequire(import.meta.url);
const { Project, formatDiagnostics } = require("./index.cjs");

const strip = (id) => id.split("?")[0];
const SWEET = /\.stsx?$/;

function findTsconfig(dir) {
  for (let at = path.resolve(dir); ; at = path.dirname(at)) {
    const candidate = path.join(at, "tsconfig.json");
    if (existsSync(candidate)) return candidate;
    if (path.dirname(at) === at) return undefined;
  }
}

export const unplugin = createUnplugin((options = {}, meta) => {
  const { tsconfig, stripTypes = true, ...categories } = options;
  let root = process.cwd();
  let project;
  let viteConfig;

  function open() {
    if (project) return project;
    const config = tsconfig ? path.resolve(root, tsconfig) : findTsconfig(root);
    if (!config) throw new Error(`@effect-x/tsc: no tsconfig.json at or above ${root}`);
    project = Project.open({ tsconfig: config, ...categories });
    return project;
  }

  function report(ctx, file, result) {
    for (const dependency of result.dependencies) ctx.addWatchFile?.(dependency);
    for (const item of result.warnings) {
      ctx.warn({ message: formatDiagnostics([item]), id: file, pos: item.start });
    }
    if (result.errors.length > 0) {
      ctx.error({ message: formatDiagnostics(result.errors), id: file, pos: result.errors[0].start });
    }
  }

  /** Owned files whose output changed with `file`, as invalidated. */
  function invalidate(file) {
    return project ? project.invalidate(path.resolve(file)) : [];
  }

  return {
    name: "@effect-x/tsc",
    enforce: "pre",

    buildStart() {
      const current = open();
      for (const message of current.configErrors) this.warn(message);
    },

    // `.sts`/`.stsx` keep their real path; an extensionless relative import
    // finds them when no .ts/.tsx/.js/.jsx does.
    resolveId(source, importer) {
      if (!importer || !(source.startsWith("./") || source.startsWith("../"))) return null;
      const target = path.resolve(path.dirname(strip(importer)), strip(source));
      if (SWEET.test(target)) return existsSync(target) ? target : null;
      if (path.extname(target)) return null;
      for (const ext of [".ts", ".tsx", ".js", ".jsx"]) {
        if (existsSync(target + ext)) return null;
      }
      for (const ext of [".stsx", ".sts"]) {
        if (existsSync(target + ext)) return target + ext;
      }
      return null;
    },

    loadInclude(id) {
      return !id.startsWith("\0") && open().owns(strip(id));
    },

    async load(id) {
      const file = strip(id);
      const result = await open().getFileAsync(file);
      report(this, file, result);
      return { code: result.code, map: result.map, moduleType: result.moduleType };
    },

    watchChange(id) {
      invalidate(strip(id));
    },

    buildEnd() {
      if (meta.watchMode) return;
      project?.dispose();
      project = undefined;
    },

    vite: {
      config(config) {
        root = config.root ? path.resolve(config.root) : process.cwd();
      },
      configResolved(config) {
        root = config.root;
        viteConfig = config;
      },
      // vite:oxc picks the language from the file extension and does not
      // know .sts/.stsx (nor read `moduleType`), so strip their types here
      // with Vite's own oxc options (JSX runtime, refresh) and lang ts/tsx.
      async transform(code, id) {
        const file = strip(id);
        if (!stripTypes || !SWEET.test(file) || viteConfig?.oxc === false) return null;
        // The app's vite, not one next to this package.
        const vitePath = createRequire(path.join(viteConfig.root, "package.json")).resolve("vite");
        const { transformWithOxc } = await import(pathToFileURL(vitePath).href);
        const { include, exclude, jsxInject, jsxRefreshInclude, jsxRefreshExclude, ...oxc } = viteConfig?.oxc ?? {};
        const lang = file.endsWith("x") ? "tsx" : "ts";
        const result = await transformWithOxc(code, `${file}.${lang}`, { ...oxc, lang, sourcemap: true }, undefined, viteConfig);
        if (jsxInject && lang === "tsx") result.code = `${jsxInject};${result.code}`;
        return { code: result.code, map: result.map, moduleType: "js" };
      },
      // Vite's dev server serves an unknown extension as a static file unless
      // the request is a module import; mark entry scripts as one.
      transformIndexHtml(html) {
        return html.replace(/(<script\b[^>]*\bsrc="[^"?]+\.stsx?)"/g, '$1?import"');
      },
      hotUpdate({ file, modules }) {
        const affected = invalidate(file);
        if (affected.length === 0) return;
        const graph = this.environment.moduleGraph;
        const out = new Set(modules);
        for (const owned of affected) {
          for (const mod of graph.getModulesByFile(owned) ?? []) {
            graph.invalidateModule(mod);
            out.add(mod);
          }
        }
        return [...out];
      },
    },
  };
});

export default unplugin;
export const vite = unplugin.vite;
export const rollup = unplugin.rollup;
export const rolldown = unplugin.rolldown;
export const webpack = unplugin.webpack;
export const rspack = unplugin.rspack;
export const esbuild = unplugin.esbuild;

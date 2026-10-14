import type { UnpluginInstance } from "unplugin";
import type { TransformOptions } from "./index.cjs";

export interface PluginOptions extends TransformOptions {
  /** tsconfig.json of the project. Default: the nearest one at or above the bundler root. */
  tsconfig?: string;
  /**
   * Vite: strip `.sts`/`.stsx` to JavaScript with Vite's oxc options (vite:oxc
   * itself skips unknown extensions). `false` when a later plugin compiles
   * the TS/TSX itself (it gets the code with `moduleType` "ts"/"tsx").
   * Default `true`.
   */
  stripTypes?: boolean;
}

/**
 * Loads the project's own files (tsconfig include/files, minus node_modules
 * and declarations) through `Project`: `.sts`/`.stsx` and `.ts`/`.tsx` come
 * out as TS/TSX (`moduleType`) with an exact map. Put it before other plugins.
 *
 * Vite/Rolldown/Rollup: complete (Vite's oxc include gains .sts/.stsx).
 * webpack/rspack: the output is TypeScript; add a TS loader rule for
 * /\.stsx?$/ (ts-loader, builtin:swc-loader). esbuild/bun: unsupported until
 * type erasure lands (esbuild cannot pick a loader for .sts).
 */
export declare const unplugin: UnpluginInstance<PluginOptions | undefined, false>;
export default unplugin;
export declare const vite: UnpluginInstance<PluginOptions | undefined, false>["vite"];
export declare const rollup: UnpluginInstance<PluginOptions | undefined, false>["rollup"];
export declare const rolldown: UnpluginInstance<PluginOptions | undefined, false>["rolldown"];
export declare const webpack: UnpluginInstance<PluginOptions | undefined, false>["webpack"];
export declare const rspack: UnpluginInstance<PluginOptions | undefined, false>["rspack"];
export declare const esbuild: UnpluginInstance<PluginOptions | undefined, false>["esbuild"];

// The @effect-x/tsc Node API.
//
// One pipeline for dev and build: Sweetener expansion (for .sts/.stsx), then
// TypeGPU in its build form (operators rewritten, "use gpu" functions wrapped
// with __TYPEGPU_META__), as unplugin-typegpu does in a bundler. Any other
// file runs the TypeGPU step only. To leave a file alone, do not call
// transform on it.
//
// .sjs/.sjsx (Sweetener over JavaScript) are not supported: transform throws.
//
// Options shape the output of this API only (the runtime JavaScript). `tsc`
// checks the Sweetener expansion as TypeScript; no option changes it.

/** Sweetener options. */
export interface SweetenerOptions {
  /**
   * Drop `.sts`/`.stsx` from module specifiers (`"./logic.sts"` → `"./logic"`), for a resolver
   * that finds the virtual `.ts` by extension search. Default `false`: specifiers stay as written.
   */
  rewriteImports?: boolean;
}

/** TypeGPU options, named as in unplugin-typegpu. */
export interface TypegpuOptions {
  /** Name TypeGPU resources after their binding. Default `true`. */
  autoNaming?: boolean;
  /** Skip files that do not import TypeGPU. Default `false`. */
  earlyPruning?: boolean;
  /** The name `tgpu` is imported as. Default: detected from imports. */
  forceTgpuAlias?: string;
}

/**
 * Options by category. The same shape is the `options` object of the
 * tsconfig `contentMappers` entry for "@effect-x/tsc".
 */
export interface TransformOptions {
  sweetener?: SweetenerOptions;
  typegpu?: TypegpuOptions;
}

export interface TransformerOptions extends TransformOptions {
  /**
   * tsconfig to read options from. Default: the nearest tsconfig.json above
   * each file. `false`: read no tsconfig. Explicit options override single
   * fields of the tsconfig's.
   */
  tsconfig?: string | false;
}

export interface RelatedLocation {
  message: string;
  file: string;
  /** 1-based. */
  line: number;
  /** 1-based, UTF-16 code units. */
  column: number;
  /** UTF-16 offsets into the source passed to transform. */
  start: number;
  end: number;
}

export interface Diagnostic extends RelatedLocation {
  /** The SWR number (4025 for SWR4025), 0 when the message has none. */
  code: number;
  category: "error" | "warning" | "suggestion" | "message";
  related: RelatedLocation[];
}

export interface SourceMapV3 {
  version: 3;
  file?: string;
  sources: string[];
  sourcesContent?: (string | null)[];
  names: string[];
  mappings: string;
}

export interface Result {
  /** TypeScript (or TSX) for the rest of the pipeline. */
  code: string;
  /** The filename for `code`: `a.sts` → `a.ts`, `a.stsx` → `a.tsx`; others unchanged. */
  hostPath: string;
  /** Absolute paths of files the expansion read (macro modules). Watch them. */
  dependencies: string[];
  /** Every diagnostic, in source order. */
  diagnostics: Diagnostic[];
  /** The diagnostics whose category is "error". A bundler fails the file on any. */
  errors: Diagnostic[];
  /** The other diagnostics (warnings, suggestions, messages). */
  warnings: Diagnostic[];
  /**
   * `code` → the file passed to transform. Always a map: when `changed` is
   * false it is the identity map (each line to itself under `path`).
   */
  map: SourceMapV3;
  changed: boolean;
  /** The tsconfig the options came from, or null. */
  tsconfig: string | null;
  /**
   * A map of a later transform of `code`, composed onto the original file.
   * Exact: a position in copied text maps to the same character; a
   * position in macro output maps to the start of the author text that
   * produced it; text the expansion introduced is unmapped. With several
   * sources, the one named `hostPath` (or its basename) is `code`; the
   * others are kept. Unchanged files compose through the identity map, so
   * the result always names the original `path`.
   */
  composeMap(map: SourceMapV3 | string): SourceMapV3;
}

export declare class Transformer {
  constructor(options?: TransformerOptions);
  /** `code` is the text of `path`; the file is not read. */
  transform(code: string, path: string, options?: TransformerOptions): Result;
  /** Read `path` (UTF-8) and transform it. */
  transformFile(path: string, options?: TransformerOptions): Result;
}

/**
 * Diagnostics as text, one per line:
 * `file(line,column): category SWRcode: message`, related locations indented.
 */
export declare function formatDiagnostics(diagnostics: readonly Diagnostic[]): string;

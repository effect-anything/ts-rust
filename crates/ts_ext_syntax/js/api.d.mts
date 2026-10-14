// "@effect-x/tsc/unstable/mapped": what a content mapper (Sweetener for
// .sts/.stsx) did to the files of a program. See api.mjs.

/** A file as the unstable API names one: a file name or a URI. */
export type DocumentIdentifier = string | { uri: string };

/**
 * How a mapped position relates to its counterpart:
 * - `exact`: 1:1 (user-written text; safe to edit through),
 * - `atom`: inside macro output, mapped to its whole original range,
 * - `approximate`: across segment boundaries,
 * - `none`: no counterpart (synthesized text).
 */
export type Fidelity = "exact" | "atom" | "approximate" | "none";

export interface MappedPosition {
  readonly position: number;
  readonly fidelity: Fidelity;
}

/** One `import { ... } from "m" for syntax;` of the original file. */
export interface SyntaxImport {
  /** The specifier as written, without quotes. */
  readonly specifier: string;
  /** The macro module's canonical path; absent when it does not resolve. */
  readonly resolved?: string;
  readonly bindings: readonly { readonly imported: string; readonly local: string }[];
  readonly shadowsCore: boolean;
  /** The whole declaration in the original text. */
  readonly originalStart: number;
  readonly originalEnd: number;
}

/** One macro invocation (original range) and the virtual text it produced. */
export interface Expansion {
  readonly virtualStart: number;
  readonly virtualEnd: number;
  readonly originalStart: number;
  readonly originalEnd: number;
  /** e.g. "macro `twice` from \"./macros.sts\"". */
  readonly label: string;
}

/**
 * A run of the virtual text: `verbatim` is user-written (maps 1:1),
 * `generated` is macro output attributed to its invocation, `synthesized`
 * has no original counterpart (no `original*` members).
 */
export interface Region {
  readonly virtualStart: number;
  readonly virtualEnd: number;
  readonly kind: "verbatim" | "generated" | "synthesized";
  readonly originalStart?: number;
  readonly originalEnd?: number;
}

export interface MappedFileInfo {
  readonly fileName: string;
  /** False for a file no content mapper produced (identity regions). */
  readonly mapped: boolean;
  readonly contentMapper: string;
  /** Every file the transform read (`for syntax` modules and their imports), sorted. */
  readonly dependencies: readonly string[];
  /** The file's syntax imports, which the virtual AST no longer contains. */
  readonly syntaxImports: readonly SyntaxImport[];
  readonly expansions: readonly Expansion[];
  /** The virtual text, partitioned. */
  readonly regions: readonly Region[];
}

/** `T` for the sync client, `Promise<T>` for the async one. */
type Result<P, T> = P extends { readonly client: { apiRequest(...args: any[]): Promise<any> } } ? Promise<T> : T;

export interface MappedFiles<P> {
  /** `undefined` when the program has no such file. */
  getMappedFileInfo(file: DocumentIdentifier): Result<P, MappedFileInfo | undefined>;
  getMappedFileInfo(files: readonly DocumentIdentifier[]): Result<P, (MappedFileInfo | undefined)[]>;
  /** Every content-mapped file of the program. */
  getMappedFilesInfo(): Result<P, MappedFileInfo[]>;
  /** Virtual → original. */
  getOriginalPosition(file: DocumentIdentifier, position: number): Result<P, MappedPosition | undefined>;
  getOriginalPosition(file: DocumentIdentifier, positions: readonly number[]): Result<P, MappedPosition[] | undefined>;
  /** Original → virtual: every projection (macro output may repeat an argument). */
  getVirtualPositions(file: DocumentIdentifier, position: number): Result<P, MappedPosition[] | undefined>;
  getVirtualPositions(file: DocumentIdentifier, positions: readonly number[]): Result<P, MappedPosition[][] | undefined>;
}

/** Pass a `Program` or `Project` from "@effect-x/tsc/unstable/sync" or ".../async". */
export declare function mappedFiles<P extends object>(programOrProject: P): MappedFiles<P extends { readonly program: infer Q } ? Q : P>;

/** Whether a virtual range lies entirely in user-written text. */
export declare function isUserWritten(info: MappedFileInfo, start: number, end?: number): boolean;

/** The innermost expansion containing a virtual range, if any. */
export declare function expansionAt(info: MappedFileInfo, start: number, end?: number): Expansion | undefined;

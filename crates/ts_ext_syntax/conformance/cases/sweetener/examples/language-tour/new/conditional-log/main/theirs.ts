

export function report(verbose: boolean, value: string): string {
  if (verbose) { globalThis.console.log(value); }
  return value;
}

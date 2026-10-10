/** Called by the macro templates below, from the module that defines them. */
export function helper(value: number): string {
  return `module helper saw ${String(value)}`;
}

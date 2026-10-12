export {};

declare global {
  var __TYPEGPU_META__: WeakMap<object, unknown> | undefined;
  var __TYPEGPU_AUTONAME__:
    | ((value: unknown, name: string) => unknown)
    | undefined;

  function __tsover_add(a: number, b: number): number;
  function __tsover_sub(a: number, b: number): number;
  function __tsover_mul(a: number, b: number): number;
  function __tsover_div(a: number, b: number): number;
  function __tsover_mod(a: number, b: number): number;
  function __tsover_pow(a: number, b: number): number;
}

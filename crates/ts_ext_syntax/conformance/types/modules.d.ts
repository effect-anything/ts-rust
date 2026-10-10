declare namespace Effect {
  const succeed: any;
  const gen: any;
  type Effect<A = any, E = any, R = any> = A | E | R;
}
declare module "*.css" {
  const css: string;
  export default css;
}
declare module "effect" {
  export const Effect: any;
  export const Context: any;
  export const Data: any;
  export const Schema: any;
  export const Layer: any;
  export const pipe: any;
  const effect: any;
  export default effect;
}
declare module "react" {
  export const c: any;
  export function useState(initial: any): any;
  export function useMemo(factory: any, deps: any): any;
  export function useCallback(fn: any, deps: any): any;
  export function useContext(context: any): any;
  export function useDeferredValue(value: any): any;
  export function useEffect(effect: any, deps?: any): any;
  export function useRef(value: any): any;
  export function useId(): any;
  export function createContext(value: any): any;
  const React: any;
  export default React;
}
declare module "react/compiler-runtime" {
  export function c(size: number): any;
}
declare module "react/jsx-runtime" {
  export function jsx(type: any, props: any, key?: any): any;
  export function jsxs(type: any, props: any, key?: any): any;
  export const Fragment: any;
}
declare module "react-dom" {
  const ReactDOM: any;
  export default ReactDOM;
}
declare module "react-dom/client" {
  export function createRoot(container: any): any;
}
declare module "typegpu" {
  export const tgpu: any;
  export const d: any;
  export function fn(params: any, body: any): any;
  const typegpu: any;
  export default typegpu;
}
declare module "drizzle-orm" {
  export const sql: any;
  export const eq: any;
  export const and: any;
  export const or: any;
  export const asc: any;
  export const desc: any;
  export const gt: any;
  export const lt: any;
  export const ne: any;
  const drizzle: any;
  export default drizzle;
}
declare module "drizzle-orm/pg-core" {
  export const pgTable: any;
  export const text: any;
  export const integer: any;
  export const serial: any;
  export const boolean: any;
  export const varchar: any;
  export const timestamp: any;
  export const uuid: any;
  const core: any;
  export default core;
}
declare function __tsover_add(a: any, b: any): any;
declare function __tsover_sub(a: any, b: any): any;
declare function __tsover_mul(a: any, b: any): any;
declare function __tsover_div(a: any, b: any): any;
declare function __tsover_mod(a: any, b: any): any;

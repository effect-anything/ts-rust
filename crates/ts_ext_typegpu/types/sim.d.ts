// The cases are not installed against the published package. These names
// exist so check can see our rewrite, not a missing `typegpu` install.
// Do not declare `fn` or `hello`: some snippets bind those names themselves.

type TypegpuAny = {
  (...args: any[]): TypegpuAny;
  [key: string]: any;
  new (...args: any[]): TypegpuAny;
};

declare const tgpu: TypegpuAny;
declare const d: TypegpuAny;
declare const root: TypegpuAny;
declare const u32: TypegpuAny;
declare const vec3f: TypegpuAny;
declare const GPUBufferUsage: TypegpuAny;

declare module "typegpu" {
  export const tgpu: TypegpuAny;
  export const d: TypegpuAny;
  export type TgpuUniform<T = any> = T;
  const mod: TypegpuAny;
  export default mod;
}

declare module "typegpu/data" {
  export const struct: TypegpuAny;
  export const arrayOf: TypegpuAny;
  export const u32: TypegpuAny;
  export const i32: TypegpuAny;
  export const f32: TypegpuAny;
  export const vec2u: TypegpuAny;
  export const vec3f: TypegpuAny;
  export const vec4f: TypegpuAny;
  export type U32 = any;
  export const builtin: TypegpuAny;
  const data: TypegpuAny;
  export default data;
}

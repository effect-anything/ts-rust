import { tgpu } from 'typegpu';
const root = await tgpu.init();
const myGuardedPipeline = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createGuardedComputePipeline(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
'use gpu';
}, {
v: 2,
name: undefined,
ast: {
params: [],
body: [0, []]
},
externals: {}
}) && $.f)({})), "myGuardedPipeline");
const anotherGuardedPipeline = /*#__PURE__*/(globalThis.__TYPEGPU_AUTONAME__ ?? (a => a))(root.createGuardedComputePipeline(/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = () => {
'use gpu';
}, {
v: 2,
name: undefined,
ast: {
params: [],
body: [0, []]
},
externals: {}
}) && $.f)({})).dispatchThreads(), "anotherGuardedPipeline");
console.log(myGuardedPipeline, anotherGuardedPipeline);

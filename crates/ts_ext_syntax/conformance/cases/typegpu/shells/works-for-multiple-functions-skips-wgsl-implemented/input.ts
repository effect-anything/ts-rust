import { tgpu } from 'typegpu';

const a = tgpu.computeFn({ workgroupSize: [1] })((input) => {
  const x = true;
});

const b = tgpu.fn([])(() => {
  const y = 2 + 2;
});

const cx = 2;
const c = tgpu.fn([])(() => cx);

const d = tgpu.fn([])('() {}');

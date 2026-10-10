import { tgpu } from 'typegpu';

  const fun = tgpu.computeFn({ workgroupSize: [1] })((input) => {
    const x = true;
  });

  const funcWithAs = tgpu.computeFn({ workgroupSize: [1] })((input) => {
    const x = true as boolean;
  });

  const funcWithSatisfies = tgpu.computeFn({ workgroupSize: [1] })((input) => {
    const x = true satisfies boolean;
  });

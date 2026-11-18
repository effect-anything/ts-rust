      import { tgpu } from 'typegpu';

      const root = await tgpu.init();

      const myGuardedPipeline = root.createGuardedComputePipeline(() => {
        'use gpu';
      });

      const anotherGuardedPipeline = root
        .createGuardedComputePipeline(() => {
          'use gpu';
        }).dispatchThreads();

      console.log(myGuardedPipeline, anotherGuardedPipeline);
    

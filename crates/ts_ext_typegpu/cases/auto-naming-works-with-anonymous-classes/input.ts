      const cls = new (class {
        myConst = tgpu.const(d.u32, 0);
        #const = tgpu.const(d.u32, 1);
      })();
      console.log(cls);
    

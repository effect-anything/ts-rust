    const ext = {
      value: 7,
      config: {
        multiplier: 1,
        zero: 0,
      }
    };
    const fn = () => {
      'use gpu';
      const a = ext.value;
      const b = ext.config.multiplier;
      const c = ext.config.zero;
      const d = ext.config.multiplier;
    };
    console.log(fn);

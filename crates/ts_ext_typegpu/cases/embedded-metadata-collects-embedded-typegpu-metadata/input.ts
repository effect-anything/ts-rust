    const fn1 = () => {
      'use gpu';
    };

    const fn2 = () => {
      'use gpu';
      'worklet';
    };

    const fn3 = () => {
      'worklet';
      'use gpu';
    };

    console.log(fn1, fn2, fn3);

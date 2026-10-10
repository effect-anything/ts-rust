// extracted from use-gpu-directive.test.ts pin fc85681e title: hoists function statements marked with "use gpu" that shadow their own name
    console.log(miter);

    function miter(a) {
      'use gpu';
      const miter = a * 2;
      return miter;
    }
  

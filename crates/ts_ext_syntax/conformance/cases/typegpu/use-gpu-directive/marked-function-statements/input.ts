// extracted from use-gpu-directive.test.ts pin fc85681e title: marked function statements

    /** ADD */
    function addGPU(a, b) {
      'use gpu';
      // hello there
      return a + b;
    }


    function addCPU(a, b) {
      return a + b;
    }

    console.log(addGPU(1, 2), addCPU(2, 4));
  

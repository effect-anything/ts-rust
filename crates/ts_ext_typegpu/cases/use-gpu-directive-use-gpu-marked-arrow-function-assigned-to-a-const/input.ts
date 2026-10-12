// extracted from use-gpu-directive.test.ts pin fc85681e title: "use gpu" marked arrow function, assigned to a const
    /** ADD */
    // another comment
    const addGPU = (a, b) => {
      'use gpu';
      return a + b;
    };

    const addCPU = (a, b) => {
      return a + b;
    };

    console.log(addGPU, addCPU);
  

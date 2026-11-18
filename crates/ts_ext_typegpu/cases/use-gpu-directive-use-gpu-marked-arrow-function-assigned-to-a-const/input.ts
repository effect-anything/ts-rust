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
  

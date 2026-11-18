    const c = 1 + 2;
    if (globalThis.YUP) {
      console.log(add, mul);

      /** ADD */
      // another comment
      function add(a, b) {
        'use gpu';
        return a + b + c;
      }

      /** MUL */
      // another comment
      function mul(a, b) {
        'use gpu';
        return a * b * c;
      }
    }
  

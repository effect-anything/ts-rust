// extracted from use-gpu-directive.test.ts pin fc85681e title: hoists function statements marked with "use gpu", scoped inside an if statement
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
  

// extracted from use-gpu-directive.test.ts pin fc85681e title: replaces function statements marked with "use gpu" in place when conditions aren't ideal (like within switch statements)
    const c = 1 + 2;
    switch (globalThis.YUP) {
      case 0:
        console.log(add, mul);

        /** ADD */
        // another comment
        function add(a, b) {
          'use gpu';
          return a + b + c;
        }
        break;
      default:
        /** MUL */
        // another comment
        function mul(a, b) {
          'use gpu';
          return a * b * c;
        }
        break;
    }
  

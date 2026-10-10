// extracted from use-gpu-directive.test.ts pin fc85681e title: hoists global function statements marked with "use gpu"
    console.log(add, mul);

    /** ADD */
    // another comment
    function add(a, b) {
      'use gpu';
      return a + b;
    }

    /** MUL */
    // another comment
    function mul(a, b) {
      'use gpu';
      return a * b;
    }

  

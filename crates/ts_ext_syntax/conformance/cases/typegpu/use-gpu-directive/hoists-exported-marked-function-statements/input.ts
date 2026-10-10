// extracted from use-gpu-directive.test.ts pin fc85681e title: hoists exported marked function statements
    console.log(add);
    console.log(mul);

    /** ADD */
    export function add(a, b) {
      'use gpu';
      return a + b;
    }

    /** MUL */
    export function mul(a, b) {
      'use gpu';
      return a * b;
    }
  

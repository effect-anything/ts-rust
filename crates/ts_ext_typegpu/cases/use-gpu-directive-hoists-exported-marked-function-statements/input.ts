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
  

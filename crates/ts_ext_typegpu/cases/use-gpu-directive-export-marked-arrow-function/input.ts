    export const add = (a, b) => {
      'use gpu';
      return a + b;
    };

    const increment = (n) => {
      'use gpu';
      return n + 1;
    };

    export { increment };
    
    const mul = (a, b) => {
      'use gpu';
      return a * b;
    };

    export default mul;
  

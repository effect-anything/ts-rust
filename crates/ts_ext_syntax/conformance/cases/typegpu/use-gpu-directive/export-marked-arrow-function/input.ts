// extracted from use-gpu-directive.test.ts pin fc85681e title: export marked arrow function
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
  

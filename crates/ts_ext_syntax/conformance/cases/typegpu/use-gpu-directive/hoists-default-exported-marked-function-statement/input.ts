// extracted from use-gpu-directive.test.ts pin fc85681e title: hoists default exported marked function statement
    console.log(add);

    /** ADD */
    export default function add(a, b) {
      'use gpu';
      return a + b;
    }
  

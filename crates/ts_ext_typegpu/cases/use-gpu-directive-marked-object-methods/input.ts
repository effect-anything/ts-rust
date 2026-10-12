// extracted from use-gpu-directive.test.ts pin fc85681e title: marked object methods
    const obj = {
      /** MOD */
      mod: (a, b) => {
        'use gpu';
        return a % b;
      }
    }

    /** PRIME */
    const isPrime = (n) => {
      'use gpu';
      if (n <= 1) {
        return false;
      }

      for (let i = 2; i < n; i++) {
        if (obj.mod(n, i) === 0) {
          return false;
        }
      }
      return true;
    }

    console.log(obj, isPrime);
  

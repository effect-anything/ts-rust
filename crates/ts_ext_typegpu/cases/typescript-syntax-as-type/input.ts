
    const hello = (a: number, b: number | undefined) => {
      'use gpu';
      return a + (b as number);
    };
  

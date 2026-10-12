// extracted from typescript-syntax.test.ts pin fc85681e title: as type

    const hello = (a: number, b: number | undefined) => {
      'use gpu';
      return a + (b as number);
    };
  

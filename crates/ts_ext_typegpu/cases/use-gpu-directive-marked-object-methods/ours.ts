    const obj = {
      /** MOD */
      mod:
/*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = (a, b) => {
        'use gpu';
        return __tsover_mod(a, b);
      }, {
  v: 2,
  name: "mod",
  ast: {
    params: [{
      type: "i",
      name: "a"
    }, {
      type: "i",
      name: "b"
    }],
    body: [0, [[10, [1, "a", "%", "b"]]]]
  },
  externals: {}
}) && $.f)({})
    }

    /** PRIME */
    const isPrime = /*#__PURE__*/($ => (globalThis.__TYPEGPU_META__ ??= new WeakMap()).set($.f = n => {
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
    }, {
  v: 2,
  name: "isPrime",
  ast: {
    params: [{
      type: "i",
      name: "n"
    }],
    body: [0, [[11, [1, "n", "<=", [5, "1"]], [0, [[10, false]]]], [14, [12, "i", [5, "2"]], [1, "i", "<", "n"], [102, "++", "i"], [0, [[11, [1, [6, "obj.mod", ["n", "i"]], "===", [5, "0"]], [0, [[10, false]]]]]]], [10, true]]]
  },
  externals: {
    "obj.mod": () => obj.mod
  }
}) && $.f)({})

    console.log(obj, isPrime);

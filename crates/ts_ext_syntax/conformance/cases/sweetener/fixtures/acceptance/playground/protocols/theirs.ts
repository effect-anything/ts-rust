interface Equal<T> {
  equals(left: T, right: T): boolean;
}

const Equal = (() => {
  const implementations = new globalThis.WeakMap<object, Equal<unknown>>();
  return {
    register<ImplementationSubject>(
      prototype: object,
      implementation: Equal<ImplementationSubject>,
    ): void {
      implementations.set(prototype, implementation as Equal<unknown>);
    },
    equals<T extends object>(left: T, right: T): boolean {
      const implementation = implementations.get(
        globalThis.Object.getPrototypeOf([left, right][0]!),
      ) as Equal<T> | undefined;
      if (implementation === undefined) {
        throw new globalThis.Error("Missing " + "Equal" + " implementation");
      }
      return implementation.equals(left, right);
    },
  };
})();

class User {
  constructor(readonly id: number) {}
}

Equal.register(User.prototype, {
  equals(left: User, right: User): boolean {
    return left.id === right.id;
  },
});

export const result = Equal.equals(new User(1), new User(1));

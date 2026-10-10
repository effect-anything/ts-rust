const implementations = "outer implementations";
const implementation = "outer implementation";

interface Equal<T> {
  equals(left: T, right: T): boolean;
}

const Equal = (() => {
  const implementations_1 = new globalThis.WeakMap<object, Equal<unknown>>();
  return {
    register<ImplementationSubject>(
      prototype: object,
      implementation_1: Equal<ImplementationSubject>,
    ): void {
      implementations_1.set(prototype, implementation_1 as Equal<unknown>);
    },
    equals<T extends object>(left: T, right: T): boolean {
      const implementation_1 = implementations_1.get(
        globalThis.Object.getPrototypeOf([left, right][0]!),
      ) as Equal<T> | undefined;
      if (implementation_1 === undefined) {
        throw new globalThis.Error("Missing " + "Equal" + " implementation");
      }
      return implementation_1.equals(left, right);
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

export const hygieneResult = [
  implementations,
  implementation,
  Equal.equals(new User(1), new User(1)),
];

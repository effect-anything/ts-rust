

interface Equal<T> {
        equals(left: T, right: T): boolean.name(
            equals(left: T, right: T): boolean.parameters.name: equals(left: T, right: T): boolean.parameters.valueType
          ): equals(left: T, right: T): boolean.resultType;
      }

    const Equal = (() => {
          const implementations = new globalThis.WeakMap<object, Equal<unknown>>();
          return {
            register<ImplementationSubject>(
              prototype: object,
              implementation: Equal<ImplementationSubject>,
            ): void {
              implementations.set(
                prototype,
                implementation as Equal<unknown>,
              );
            },
            equals(left: T, right: T): boolean.name<T extends object>(
                equals(left: T, right: T): boolean.parameters.name: equals(left: T, right: T): boolean.parameters.valueType
              ): equals(left: T, right: T): boolean.resultType {
                const implementation = implementations.get(
                  globalThis.Object.getPrototypeOf([
                      equals(left: T, right: T): boolean.parameters.name
                    ][0]!)
                ) as Equal<T> | undefined;
                if (implementation === undefined) {
                  throw new globalThis.Error(
                    "Missing " + "Equal" + " implementation"
                  );
                }
                return implementation.equals(left: T, right: T): boolean.name(
                  equals(left: T, right: T): boolean.parameters.name
                );
              },
          };
        })()

class User {
  constructor(readonly id: number) {}
}

User implements Equal<User> {
  equals(left: User, right: User): boolean {
    return left.id === right.id;
  }
}

export const result = Equal.equals(new User(1), new User(1));

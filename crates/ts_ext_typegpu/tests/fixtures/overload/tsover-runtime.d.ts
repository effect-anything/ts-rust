declare global {
  interface SymbolConstructor {
    readonly deferOperation: unique symbol;
    readonly operatorPlus: unique symbol;
    readonly operatorStar: unique symbol;
  }
}

export const Operator: {
  readonly deferOperation: typeof Symbol.deferOperation;
  readonly plus: typeof Symbol.operatorPlus;
  readonly star: typeof Symbol.operatorStar;
};

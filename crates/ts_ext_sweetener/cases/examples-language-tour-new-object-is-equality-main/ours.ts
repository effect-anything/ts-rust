

export const nanEqualsItself = globalThis.Object.is(globalThis.NaN, globalThis.NaN);
export const signedZerosDiffer = !(globalThis.Object.is(0, -0));

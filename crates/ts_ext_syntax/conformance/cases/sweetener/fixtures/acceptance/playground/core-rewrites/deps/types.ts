import { exact, nanKind, result } from "./expected.js";

exact satisfies (value: number) => number;
nanKind satisfies string;
result satisfies { nanKind: string; rejectedExtraArgument: boolean };

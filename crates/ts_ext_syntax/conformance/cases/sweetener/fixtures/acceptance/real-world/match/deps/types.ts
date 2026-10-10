import { described, describe, name, named } from "./expected.js";

describe satisfies (event: {
  readonly type: "key";
  readonly key: string;
}) => string;
name satisfies (event: {
  readonly type: "key";
  readonly key: string;
}) => string;
described satisfies readonly string[];
named satisfies readonly string[];

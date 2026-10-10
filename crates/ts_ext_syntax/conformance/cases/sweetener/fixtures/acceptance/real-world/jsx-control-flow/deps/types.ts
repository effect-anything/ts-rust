import { list, rendered } from "./expected.js";

list satisfies (items: readonly never[], loading: boolean) => unknown;
rendered satisfies readonly string[];

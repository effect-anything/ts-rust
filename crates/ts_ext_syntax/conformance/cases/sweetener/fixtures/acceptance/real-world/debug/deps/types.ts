import { describeFailure, failure, inspected, total } from "./expected.js";

total satisfies (values: readonly number[]) => number;
describeFailure satisfies (values: readonly number[]) => string;
inspected satisfies string;
failure satisfies string;

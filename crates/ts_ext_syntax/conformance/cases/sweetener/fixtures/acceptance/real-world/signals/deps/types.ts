import { assigned, observed, seen, settled } from "./expected.js";

seen satisfies string[];
observed satisfies readonly string[];
assigned satisfies number;
settled satisfies readonly [number, boolean];

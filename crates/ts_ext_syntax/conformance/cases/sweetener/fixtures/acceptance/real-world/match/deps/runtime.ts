/**
 * Reached only when no arm matched. The parameter is `never`, so a match that
 * does not cover its subject is a type error at the call rather than a throw at
 * run time; the throw remains for a value that slipped past the type system.
 */
export function matchUnhandled(value: never): never {
  throw new Error(`no pattern matched: ${JSON.stringify(value)}`);
}

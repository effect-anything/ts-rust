

const readings = [3, 1, 4, 1, 5];

// `dbg` prints the expression and its value, then hands the value back, so it
// can be dropped into the middle of anything.
export const largest: number = ((inspected_1) => { globalThis.console.error(`${"Math.max(...readings)"} = ${globalThis.JSON.stringify(inspected_1)}`,); return inspected_1; })(Math.max(...readings));

// The name `inspected` that `dbg` introduces cannot collide with this one.
// Look at the generated TypeScript: yours is untouched.
export const inspected = "mine";

{
      const actual = readings.length;
      const expected = 5;
      if (!globalThis.Object.is(actual, expected)) {
        throw new globalThis.Error(
          `${"readings.length"} === ${"5"}` +
          `\n  left  = ${globalThis.JSON.stringify(actual)}` +
          `\n  right = ${globalThis.JSON.stringify(expected)}`,
        );
      }
    }

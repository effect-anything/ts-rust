

// One line each. The class, its fields, its constructor, and a printer are
// all generated — and TypeScript type-checks the result as ordinary code.
export class Point { readonly x: number;
  readonly y: number;

      constructor(x: number, y: number) { (this.x = x);
        (this.y = y);
      }

      // Generated alongside the class, from the same declaration.
      toString(): string {
        return `${"Point"}(${[String(this.x), String(this.y)].join(", ")})`;
      }
    }
export class Named { readonly label: string;
  readonly at: Point;

      constructor(label: string, at: Point) { (this.label = label);
        (this.at = at);
      }

      // Generated alongside the class, from the same declaration.
      toString(): string {
        return `${"Named"}(${[String(this.label), String(this.at)].join(", ")})`;
      }
    }

export const origin = new Point(0, 0);
export const corner = new Named("corner", new Point(3, 4));

export const printed: string = corner.toString();

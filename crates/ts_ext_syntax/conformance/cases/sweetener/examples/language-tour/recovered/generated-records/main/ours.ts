

// One line each. The class, its fields, its constructor, and a printer are
// all generated — and TypeScript type-checks the result as ordinary code.
export class Point {
      readonly x.name: x.kind;

      constructor(x.name: x.kind) {
        this.x.name = x.name;
      }

      // Generated alongside the class, from the same declaration.
      toString(): string {
        return `${"Point"}(${[$(String(this.x.name)),*].join(", ")})`;
      }
    }
export class Named {
      readonly label.name: label.kind;

      constructor(label.name: label.kind) {
        this.label.name = label.name;
      }

      // Generated alongside the class, from the same declaration.
      toString(): string {
        return `${"Named"}(${[$(String(this.label.name)),*].join(", ")})`;
      }
    }

export const origin = new Point(0, 0);
export const corner = new Named("corner", new Point(3, 4));

export const printed: string = corner.toString();

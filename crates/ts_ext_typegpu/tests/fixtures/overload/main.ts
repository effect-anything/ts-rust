"use tsover";

import { Operator } from "./tsover-runtime";

export class Vec2f {
  x: number;
  y: number;
  constructor(x: number, y: number) {
    this.x = x;
    this.y = y;
  }
  [Operator.plus](lhs: Vec2f, rhs: Vec2f): Vec2f {
    return new Vec2f(lhs.x + rhs.x, lhs.y + rhs.y);
  }
  [Operator.star](lhs: number, rhs: Vec2f): Vec2f;
  [Operator.star](lhs: Vec2f, rhs: number): Vec2f;
  [Operator.star](
    lhs: Vec2f | number,
    rhs: Vec2f | number,
  ): Vec2f | typeof Operator.deferOperation {
    if (typeof lhs === "number" && rhs instanceof Vec2f) {
      return new Vec2f(lhs * rhs.x, lhs * rhs.y);
    }
    if (typeof rhs === "number" && lhs instanceof Vec2f) {
      return new Vec2f(lhs.x * rhs, lhs.y * rhs);
    }
    return Operator.deferOperation;
  }
}

export function total(a: Vec2f, b: Vec2f) {
  const sum: Vec2f = a + b;
  const scaled: Vec2f = a * 2;
  const fromLeft: Vec2f = 2 * b;
  const notNumber: number = a + b;
  const bad = a + 1;
  const n = 1 + 2;
  return sum + scaled + fromLeft;
}

export function marked(a: Vec2f, b: Vec2f) {
  "use gpu";
  return a + b;
}

export const plain = 1 + 2;

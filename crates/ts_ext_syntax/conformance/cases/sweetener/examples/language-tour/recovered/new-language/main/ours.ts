

export class Empty {}
export class Cons {}

function last(self: Cons): number { return self.tail instanceof Empty ? self.head : last(self.tail); }

const list: Cons | undefined = new Cons(
  1,
  new Cons(3, new Empty()),
);
export const result = last()(list);

const self = "outer self";
const body = "outer body";

namespace List {
  export class Empty {}
  export class Cons {
    head: number;
    tail: Empty | Cons;
    constructor(head: number, tail: Empty | Cons) {
      this.head = head;
      this.tail = tail;
    }
  }
}

function last(self_1: List.Cons): number {
  return self_1.tail instanceof List.Empty ? self_1.head : last(self_1.tail);
}

const list = new List.Cons(1, new List.Cons(3, new List.Empty()));
export const hygieneResult = [self, body, last(list)];

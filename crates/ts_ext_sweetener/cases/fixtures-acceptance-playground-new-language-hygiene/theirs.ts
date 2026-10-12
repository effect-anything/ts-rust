

const self = "outer self";
const body = "outer body";

module List {
  export class Empty {}
  export class Cons { head: number;
    tail: (Empty | Cons);
      constructor(head: number, tail: (Empty | Cons)) { (this.head = head);
        (this.tail = tail);
      }
    }
}

function last(self: List.Cons): number
{
    return (self.tail instanceof List.Empty ? self.head : last(self.tail));
}

const list = new List.Cons(1, new List.Cons(3, new List.Empty()));
export const hygieneResult = [self, body, last(list)];

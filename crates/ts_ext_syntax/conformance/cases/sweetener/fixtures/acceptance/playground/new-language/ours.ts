

namespace List {
      record Empty()
    }

function last(self: List.Cons): number { return self.tail instanceof List.Empty ? self.head : last(self.tail); }

const list: List.Cons | undefined = new List.Cons(
  1,
  new List.Cons(3, new List.Empty()),
);
export const result = last() :: list;

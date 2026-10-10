type Box<A> = { readonly value: A };
const box = {
  of: <A>(value: A): Box<A> => ({ value }),
  flatMap: <A, B>(source: Box<A>, next: (value: A) => Box<B>): Box<B> =>
    next(source.value),
};

const left = "outer left";
const source = "outer source";

export const hygieneResult = {
  outer: [left, source],
  nested: box.flatMap(box.of(4), (left_1) =>
    box.flatMap(box.of(6), (source_1) => box.of(left_1 + source_1)),
  ),
};

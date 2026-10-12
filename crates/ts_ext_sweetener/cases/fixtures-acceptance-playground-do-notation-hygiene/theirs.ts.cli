

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
  nested: box.flatMap(box.of(4), (left) => box.flatMap(box.of(6), (source) => box.of(left + source)
    )
    ),
};

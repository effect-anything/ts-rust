

type Box<A> = { readonly value: A };
const box = {
  of: <A,>(value: A): Box<A> => ({ value }),
  flatMap: <A, B>(source: Box<A>, next: (value: A) => Box<B>): Box<B> =>
  next(source.value),
};

export const result = box.flatMap(box.all([left.sources]), ([left.names]) =>
      box.flatMap(box.all([<-.sources]), ([<-.names]) =>
      box.flatMap(box.all([box.sources]), ([box.names]) =>
      box.flatMap(box.all([..sources]), ([..names]) =>
      box.flatMap(box.all([of.sources]), ([of.names]) =>
      doSteps(box) { (2) }
    )
    )
    )
    )
    );

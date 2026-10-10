

type Box<A> = { readonly value: A };
const box = {
  of: <A,>(value: A): Box<A> => ({ value }),
  flatMap: <A, B>(source: Box<A>, next: (value: A) => Box<B>): Box<B> =>
  next(source.value),
};

export const result = (box.of(2), (left) =>
      doSteps(box) { const right = yield take(box).of(3) }
    );

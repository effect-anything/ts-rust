type Effect =
  { readonly kind: "put"; readonly value: number } | { readonly kind: "take" };
const table = {};
const put = (_channel: object, value: number): Effect => ({
  kind: "put",
  value,
});
const take = (channel: object): Effect => {
  void channel;
  return { kind: "take" };
};

function* exchange(): Generator<Effect, number, number> {
  yield put(table, 2);
  const received = yield take(table);
  return received + 1;
}

const iterator = exchange();
iterator.next();
iterator.next(0);
const final = iterator.next(2);
if (!final.done) throw new Error("exchange did not finish");
export const result = final.value;

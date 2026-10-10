type Effect =
  { readonly kind: "put"; readonly value: number } | { readonly kind: "take" };
const put = (_channel: object, value: number): Effect => ({
  kind: "put",
  value,
});
const take = (channel: object): Effect => {
  void channel;
  return { kind: "take" };
};

const received = "outer received";
const channel = {};

function* exchange(): Generator<Effect, number, number> {
  yield put(channel, 4);
  const received_1 = yield take(channel);
  return received_1 + 1;
}

export const hygieneResult = [received, exchange];

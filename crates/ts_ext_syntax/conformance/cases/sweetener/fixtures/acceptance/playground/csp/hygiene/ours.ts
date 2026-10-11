

const received = "outer received";
const channel = {};

function* exchange(): Generator<Effect, number, number> {
  yield put(channel, 4);
  const received_1 = yield take(channel);
  return received_1 + 1;
}

export const hygieneResult = [received, exchange];

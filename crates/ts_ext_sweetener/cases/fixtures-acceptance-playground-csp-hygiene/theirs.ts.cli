

const received = "outer received";
const channel = {};

function* exchange(): Generator<Effect, number, number> { (yield put(channel, 4));
  const received = (yield take(channel));
  return received + 1;
}

export const hygieneResult = [received, exchange];

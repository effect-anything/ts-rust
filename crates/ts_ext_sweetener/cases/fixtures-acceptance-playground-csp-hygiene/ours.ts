

const received = "outer received";
const channel = {};

function* exchange(): Generator<Effect, number, number> {
  4 -> channel;
  received_1 <- channel;
  return received_1 + 1;
}

export const hygieneResult = [received, exchange];

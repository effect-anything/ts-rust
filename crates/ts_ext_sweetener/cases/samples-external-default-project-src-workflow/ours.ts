

export class OrderPlaced {
      : ;
      constructor(: ) {
        this. = ;
      }
    }

const cents = (value: number) => Math.round(value * 100);

export const order: OrderPlaced | undefined = new OrderPlaced("A-17", 42.5);
export const audit = [order.id, order.total];
export const transformed = cents(order.total);
export const copies = [(order.id), (order.id)];

export function readyToShip(candidate: OrderPlaced): string {
  return `${candidate.id}:${cents(candidate.total)}`;
}

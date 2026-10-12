

export function withdraw(balance: number, amount: number): string {
  if (!(amount <= balance)) { return "insufficient funds"; }
  return `withdrew ${String(amount)}`;
}

export const denied: string = withdraw(20, 50);
export const allowed: string = withdraw(100, 50);

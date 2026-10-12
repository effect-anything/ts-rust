import { createEffect, createSignal } from "./runtime.js";

// Reactive state that reads and writes like a plain variable.
const countSignal = createSignal(1);
const labelSignal = createSignal("start");

export const seen: string[] = [];

// Re-runs whenever a signal it read has been written.
createEffect(() => {
  seen.push(`${labelSignal.get()}:${countSignal.get()}`);
});

countSignal.set(5);
countSignal.set(
            countSignal.get() + 2
          );
labelSignal.set("done");

export const observed: readonly string[] = [...seen];
export const total: number = countSignal.get();

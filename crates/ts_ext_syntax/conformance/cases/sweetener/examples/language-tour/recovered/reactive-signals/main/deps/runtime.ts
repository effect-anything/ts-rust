// Ordinary TypeScript. The macros below expand into calls to this.
export interface Signal<T> {
  get(): T;
  /** Returns what was written, so an assignment still has a value. */
  set(value: T): T;
}

const listeners: (() => void)[] = [];

export function createSignal<T>(initial: T): Signal<T> {
  let current = initial;
  return {
    get: () => current,
    set: (value: T) => {
      current = value;
      for (const listener of listeners) listener();
      return value;
    },
  };
}

export function createEffect(run: () => void): void {
  listeners.push(run);
  run();
}

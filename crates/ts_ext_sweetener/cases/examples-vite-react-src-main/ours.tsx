import { useCallback, useContext, useDeferredValue, useEffect, useId, useMemo, useReducer, useRef, useState, useTransition, } from "react";
import React, { createContext } from "react";
import { createRoot } from "react-dom/client";
import "../../macro-suite/theme.css";


const PrefixContext = createContext("Sweetened");

function counterReducer(value: number, amount: number): number {
  return value + amount;
}

export function HookCarnival() {
  const [count, setCount] = useState<number>(0);
  const [step, setStep] = useState<number>(1);
  const [page, setPage] = useState(1);const [query, setQuery] = useState("");
  const [detailsOpen, setDetailsOpen] = useState(false);
    const toggleDetailsOpen = useCallback(
      () => {
        setDetailsOpen((current) => !current);
      },
      [],
    );
  const [reducedCount, dispatchCount] = useReducer(counterReducer, 0);
  const stepInput = useRef<HTMLInputElement | null>(null);
  const prefix = useContext(PrefixContext);
  const doubled: number = useMemo(() => count * 2, [count]);
  const deferredCount = useDeferredValue(count);
  const latestStep = useRef(step);
    useEffect(() => {
        latestStep.current = step;
      }, [step]);
  const stepInputId = useId();
  const [
      navigationPending,
      startNavigation,
    ] = useTransition();
  const increment = useCallback(() => { setCount((value) => value + step); }, [step]);
  const incrementWithLatest = useCallback(() => {
    setCount((value) => value + latestStep.current);
  }, []);
  const incrementLater = useCallback(async () => {
    await Promise.resolve();
    setCount((value) => value + step);
  }, [step]);
  useEffect(() => {
    document.title = `${prefix}: ${count}`;
    return () => {
      document.title = "Sweetener React hooks";
    };
  }, [count, prefix]);
  return (
    <main>
      <h1>{prefix} React hook macros</h1>
      <p className="hero">Count {count}; doubled {doubled}; deferred {deferredCount}</p>
      <p>Reducer count: {reducedCount}</p>
      <p>Search page {page}; query {query || "(empty)"}</p>
      <button type="button" onClick={() => setPage((value) => value + 1)}>
        Next search page
      </button>
      <input value={query} onChange={(event) => setQuery(event.currentTarget.value)} />
      <button type="button" onClick={increment}>Increment</button>
      <button type="button" onClick={incrementWithLatest}>
        Increment through latest ref
      </button>
      <button type="button" onClick={() => void incrementLater()}>
        Increment asynchronously
      </button>
      <button type="button" onClick={() => dispatchCount(step)}>
        Dispatch reducer
      </button>
      <label>
        Step
        <input
          id={stepInputId}
          ref={stepInput}
          type="number"
          value={step}
          onChange={(event) => setStep(Number(event.currentTarget.value))}
        />
      </label>
      <button type="button" onClick={toggleDetailsOpen}>Toggle details</button>
      <button
        type="button"
        disabled={navigationPending}
        onClick={() => startNavigation(() => setCount((value) => value + 1))}
      >
        {navigationPending ? "Updating…" : "Update in a transition"}
      </button>
      {detailsOpen ? (
        <p>Generated bindings: setDetailsOpen and toggleDetailsOpen.</p>
      ) : null}
    </main>
  );
}

export function SecondaryCounter() {
  const [count, setCount] = useState(10);
  const [detailsOpen, setDetailsOpen] = useState(true);
    const toggleDetailsOpen = useCallback(
      () => {
        setDetailsOpen((current_1) => !current_1);
      },
      [],
    );
  return (
    <section>
      <button type="button" onClick={() => setCount((value) => value + 1)}>
        Independent count: {count}
      </button>
      <button type="button" onClick={toggleDetailsOpen}>
        Independent details: {String(detailsOpen)}
      </button>
    </section>
  );
}

createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <PrefixContext value="Sweetened">
      <HookCarnival />
      <SecondaryCounter />
    </PrefixContext>
  </React.StrictMode>,
);

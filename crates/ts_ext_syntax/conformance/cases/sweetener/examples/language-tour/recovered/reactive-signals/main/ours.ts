import { createEffect, createSignal } from "./runtime.js";


// Reactive state that reads and writes like a plain variable.
const countSignal = createSignal(1);
    #syntax {
      syntax count:expr {
        rule { count = :expr } => {
          countSignal.set()
        }

        rule { count += :expr } => {
          countSignal.set(
            countSignal.get() + 
          )
        }

        rule { count -= :expr } => {
          countSignal.set(
            countSignal.get() - 
          )
        }

        // Anything else is a read.
        rule { count } => { countSignal.get() }
      }
    }
const labelSignal = createSignal("start");
    #syntax {
      syntax label:expr {
        rule { label = :expr } => {
          labelSignal.set()
        }

        rule { label += :expr } => {
          labelSignal.set(
            labelSignal.get() + 
          )
        }

        rule { label -= :expr } => {
          labelSignal.set(
            labelSignal.get() - 
          )
        }

        // Anything else is a read.
        rule { label } => { labelSignal.get() }
      }
    }

export const seen: string[] = [];

// Re-runs whenever a signal it read has been written.
createEffect(() => { seen.push(`${label}:${count}`); })

count = 5;
count += 2;
label = "done";

export const observed: readonly string[] = [...seen];
export const total: number = count;

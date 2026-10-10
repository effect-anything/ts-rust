

export type DoorState = "Closed { lock -> Locked, open -> Opened }\n  Opened { close -> Closed }\n  Locked { unlock -> Closed }"

    export type DoorEvent =
      "Closed { lock -> Locked, open -> Opened }\n  Opened { close -> Closed }\n  Locked { unlock -> Closed }"

    export interface DoorTable {
        Closed { lock -> Locked, open -> Opened }
  Opened { close -> Closed }
  Locked { unlock -> Closed }.name: { Closed { lock -> Locked, open -> Opened }
  Opened { close -> Closed }
  Locked { unlock -> Closed }.moves.event: "Closed { lock -> Locked, open -> Opened }\n  Opened { close -> Closed }\n  Locked { unlock -> Closed }" };
      }

    // The table is also the runtime transition map, so the two cannot drift.
    export const DoorMoves = {
        Closed { lock -> Locked, open -> Opened }
  Opened { close -> Closed }
  Locked { unlock -> Closed }.name: { Closed { lock -> Locked, open -> Opened }
  Opened { close -> Closed }
  Locked { unlock -> Closed }.moves.event: "Closed { lock -> Locked, open -> Opened }\n  Opened { close -> Closed }\n  Locked { unlock -> Closed }" },
      } as const

    // `event` is keyed on the state, so an event that state does not accept
    // is not a key, and the result type is the one state it moves to.
    export const stepDoor = <
      S extends keyof DoorTable,
      E extends keyof DoorTable[S],
      >(state: S, event: E): DoorTable[S][E] =>
      DoorMoves[state][
        event as keyof (typeof DoorMoves)[S]
      ] as DoorTable[S][E]

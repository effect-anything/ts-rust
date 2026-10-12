

export type DoorState = "Closed"|"Opened"|"Locked"

    export type DoorEvent =
      |||

    export interface DoorTable {
        Closed: { : , :  };Opened: { :  };Locked: { :  };
      }

    // The table is also the runtime transition map, so the two cannot drift.
    export const DoorMoves = {
        Closed: { : , :  },Opened: { :  },Locked: { :  },
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

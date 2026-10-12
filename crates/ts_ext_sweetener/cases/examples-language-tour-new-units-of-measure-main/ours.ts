

declare const Brand: unique symbol
    export type Meters = number & { readonly [Brand]: "Meters" }
    export const Meters = {
        of: (value: number): Meters => value as Meters,
        add: (left: Meters, right: Meters): Meters => (left + right) as Meters,
        sub: (left: Meters, right: Meters): Meters => (left - right) as Meters,
        scale: (value: Meters, factor: number): Meters => (value * factor) as Meters,
      }
declare const Brand: unique symbol
    export type Seconds = number & { readonly [Brand]: "Seconds" }
    export const Seconds = {
        of: (value: number): Seconds => value as Seconds,
        add: (left: Seconds, right: Seconds): Seconds => (left + right) as Seconds,
        sub: (left: Seconds, right: Seconds): Seconds => (left - right) as Seconds,
        scale: (value: Seconds, factor: number): Seconds => (value * factor) as Seconds,
      }
declare const Brand: unique symbol
    export type MetersPerSecond = number & { readonly [Brand]: "MetersPerSecond" }
    export const MetersPerSecond = {
        of: (value: number): MetersPerSecond => value as MetersPerSecond,
        add: (left: MetersPerSecond, right: MetersPerSecond): MetersPerSecond => (left + right) as MetersPerSecond,
        from: (over: Meters, under: Seconds): MetersPerSecond => (over / under) as MetersPerSecond,
        times: (rate: MetersPerSecond, under: Seconds): Meters => (rate * under) as Meters,
      }
declare const Brand: unique symbol
    export type SquareMeters = number & { readonly [Brand]: "SquareMeters" }
    export const SquareMeters = {
        of: (value: number): SquareMeters => value as SquareMeters,
        add: (left: SquareMeters, right: SquareMeters): SquareMeters => (left + right) as SquareMeters,
        from: (left: Meters, right: Meters): SquareMeters => (left * right) as SquareMeters,
        over: (area: SquareMeters, side: Meters): Meters => (area / side) as Meters,
      }

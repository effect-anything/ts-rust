
declare class Option<A> { value: A }

type F<Inner> = Array<Inner>
export interface FunctorArray {
  map<A_1, B>(value: F<A_1>, transform: (a: A_1) => B): F<B>;
}
type F_1<Inner_1> = Option<Inner_1>
export interface FunctorOption {
  map<A_2, B_1>(value_1: F_1<A_2>, transform_1: (a_1: A_2) => B_1): F_1<B_1>;
}

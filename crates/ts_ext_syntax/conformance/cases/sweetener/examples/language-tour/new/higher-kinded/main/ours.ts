
declare class Option<A> { value: A }



type F_1<Inner> = Array<Inner>
          export interface FunctorArray {
  map<A, B>(value: F_1<A>, transform: (a_1: A) => B): F_1<B>;
}
type F_2<Inner> = Option<Inner>
          export interface FunctorOption {
  map<A, B>(value: F_2<A>, transform: (a_2: A) => B): F_2<B>;
}

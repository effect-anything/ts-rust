
declare class Option<A> { value: A }



type F<Inner> = Array<Inner>
          export interface FunctorArray {
  map<A, B>(value: F<A>, transform: (a: A) => B): F<B>;
}
type F_1<Inner> = Option<Inner>
          export interface FunctorOption {
  map<A, B>(value: F_1<A>, transform: (a_1: A) => B): F_1<B>;
}

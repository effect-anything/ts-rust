
declare class Option<A> { value: A }

;
type F<Inner> = Option<Inner>
          export interface FunctorOption {
  map<A, B>(value: F<A>, transform: (a: A) => B): F<B>;
}

Functor FunctorArray for Array

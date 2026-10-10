
declare class Option<A> { value: A }

#syntax {
      syntax Functor:item {
        rule { Functor :binding for :ident; }
        bind  in following as recursive type;
        => {
          type F<Inner> = <Inner>
          export interface  {
  map<A, B>(value: F<A>, transform: (a: A) => B): F<B>;
}
        }
      }
    }

Functor FunctorArray for Array;
Functor FunctorOption for Option;

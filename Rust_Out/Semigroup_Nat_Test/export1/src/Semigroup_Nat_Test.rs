use crate::List::*;

pub trait Semigroup {
  fn plus (x0: Self, x1: Self) -> Self;
}

pub trait Monoid : Semigroup {
  fn zero () -> Self;
}

pub fn zerolist <A>
  (xs: List<A>) -> List<A>
    where
      A : Clone + Monoid
     {
    match xs{xs => mapa((move |_: A| {
A::zero()
}), xs.clone())}
  }

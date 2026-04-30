use crate::List::*;

pub trait Semigroup {
  fn plus (x0: Self, x1: Self) -> Self;
}

pub trait Monoid : Semigroup {
  fn zero () -> Self;
}

pub fn sum <A>
  (xs: List<A>) -> A
    where
      A : Clone + Monoid
     {
    match xs{xs => fold((move |a: A, b: A| {
A::plus(a.clone(), b.clone())
}), xs.clone(), A::zero())}
  }

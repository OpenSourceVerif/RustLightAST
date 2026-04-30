use crate::Option::*;

pub trait Semigroup {
  fn plus (x0: Self, x1: Self) -> Self;
}

impl<A : Clone + Semigroup>
  Semigroup for Option<A> {
  fn plus
  (x0: Option<A>, x1: Option<A>) -> Option<A>
     {
    match (x0, x1) {
      (Option::None, Option::None) => Option::None, 
      (Option::Some (x), Option::None) => Option::Some (x.clone()), 
      (Option::None, Option::Some (x)) => Option::Some (x.clone()), 
      (Option::Some (x), Option::Some (y)) => Option::Some (A::plus(x.clone(), y.clone()))
    }
  }
}

pub fn plus0_option <A>
  (x: Option<A>) -> Option<A>
    where
      A : Clone + Semigroup
     {
    match x{x => Option::plus(x.clone(), Option::None)}
  }

use crate::Option::*;

pub trait Inc {
  fn inc (x0: Self) -> Self;
}

impl<A : Clone + Inc>
  Inc for Option<A> {
  fn inc
  (x: Option<A>) -> Option<A>
     {
    match x.clone() {
      Option::None => Option::None, 
      Option::Some (a) => Option::Some (A::inc(a.clone()))
    }
  }
}

pub fn test_inc_option <A>
  (x: Option<A>) -> Option<A>
    where
      A : Clone + Inc
     {
    match x{x => Option::inc(x.clone())}
  }

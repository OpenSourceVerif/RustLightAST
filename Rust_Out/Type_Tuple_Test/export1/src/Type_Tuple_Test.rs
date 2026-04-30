use crate::Product_Type::*;

pub fn swap <A, B>
  (x0: Prod<A, B>) -> Prod<B, A>
    where
      A : Clone, 
      B : Clone
     {
    match x0{Prod::Pair (x, y) => Prod::Pair (y.clone(), x.clone())}
  }

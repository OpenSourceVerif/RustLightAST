use crate::Nat::*;

#[derive(Clone)]
pub enum List<A> { 
  Nil, 
  Cons (A, Box<List<A>>)
}

pub fn length <A>
  (x0: List<A>) -> Nat
    where
      A : Clone
     {
    match x0 {
      List::Nil => Nat::ZeroNat, 
      List::Cons (uu, box xs) => Nat::Suc (Box::new(length(xs.clone())))
    }
  }

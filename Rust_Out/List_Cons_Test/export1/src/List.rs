use crate::Nat::*;

#[derive(Clone)]
pub enum List<A> { 
  Nil, 
  Cons (A, Box<List<A>>)
}

pub fn gen_length <A>
  (n: Nat, x1: List<A>) -> Nat
    where
      A : Clone
     {
    match (n, x1) {
      (n, List::Cons (x, box xs)) => gen_length(Nat::Suc (Box::new(n.clone())), xs.clone()), 
      (n, List::Nil) => n.clone()
    }
  }

pub fn size_list <A>
  (x: List<A>) -> Nat
    where
      A : Clone
     {
    match x{x => gen_length(Nat::ZeroNat, x.clone())}
  }

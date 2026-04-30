use crate::List::*;

pub fn length1 <A>
  (xs: List<A>) -> bool
    where
      A : Clone
     {
    match xs.clone() {
      List::Nil => false, 
      List::Cons (_, box List::Nil) => true, 
      List::Cons (_, box List::Cons (_, _)) => false
    }
  }

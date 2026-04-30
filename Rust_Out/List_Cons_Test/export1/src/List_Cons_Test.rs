use crate::List::*;
use crate::Num::*;
use crate::Nat::*;

pub fn n 
  () -> Nat
     {
    size_list(List::Cons (one_nat(), Box::new(List::Cons (nat_of_num(Num::Bit0 (Box::new(Num::One))), Box::new(List::Cons (nat_of_num(Num::Bit1 (Box::new(Num::One))), Box::new(List::Nil)))))))
  }

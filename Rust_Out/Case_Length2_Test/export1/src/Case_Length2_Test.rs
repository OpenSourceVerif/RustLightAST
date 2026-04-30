use crate::List::*;
use crate::Num::*;
use crate::Int::*;

pub fn length2 
  (xs: List<Int>) -> Int
     {
    match xs.clone() {
      List::Nil => Int::ZeroInt, 
      List::Cons (x, box List::Nil) => times_int(x.clone(), Int::Pos (Num::Bit1 (Box::new(Num::One)))), 
      List::Cons (_, box List::Cons (_, _)) => Int::ZeroInt
    }
  }

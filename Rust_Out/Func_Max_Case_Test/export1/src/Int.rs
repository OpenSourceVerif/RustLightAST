use crate::Num::*;

#[derive(Clone)]
pub enum Int { 
  ZeroInt, 
  Pos (Num), 
  Neg (Num)
}

pub fn less_int 
  (x0: Int, x1: Int) -> bool
     {
    match (x0, x1) {
      (Int::Neg (k), Int::Neg (l)) => less_num(l.clone(), k.clone()), 
      (Int::Neg (k), Int::Pos (l)) => true, 
      (Int::Neg (k), Int::ZeroInt) => true, 
      (Int::Pos (k), Int::Neg (l)) => false, 
      (Int::Pos (k), Int::Pos (l)) => less_num(k.clone(), l.clone()), 
      (Int::Pos (k), Int::ZeroInt) => false, 
      (Int::ZeroInt, Int::Neg (l)) => false, 
      (Int::ZeroInt, Int::Pos (l)) => true, 
      (Int::ZeroInt, Int::ZeroInt) => false
    }
  }

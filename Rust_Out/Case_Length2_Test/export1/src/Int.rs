use crate::Num::*;

#[derive(Clone)]
pub enum Int { 
  ZeroInt, 
  Pos (Num), 
  Neg (Num)
}

pub fn times_int 
  (k: Int, l: Int) -> Int
     {
    match (k, l) {
      (Int::Neg (m), Int::Neg (n)) => Int::Pos (times_num(m.clone(), n.clone())), 
      (Int::Neg (m), Int::Pos (n)) => Int::Neg (times_num(m.clone(), n.clone())), 
      (Int::Pos (m), Int::Neg (n)) => Int::Neg (times_num(m.clone(), n.clone())), 
      (Int::Pos (m), Int::Pos (n)) => Int::Pos (times_num(m.clone(), n.clone())), 
      (Int::ZeroInt, l) => Int::ZeroInt, 
      (k, Int::ZeroInt) => Int::ZeroInt
    }
  }

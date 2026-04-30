use crate::Num::*;
use crate::Int::*;

pub fn closure_2 
  () -> Int
     {
    plus_int(Int::Pos (Num::Bit0 (Box::new(Num::Bit1 (Box::new(Num::Bit0 (Box::new(Num::One))))))), one_int())
  }

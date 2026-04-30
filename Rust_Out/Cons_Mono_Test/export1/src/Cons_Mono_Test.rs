use crate::Num::*;
use crate::Int::*;

pub fn c 
  () -> Int
     {
    Int::Pos (Num::Bit0 (Box::new(Num::Bit1 (Box::new(Num::Bit0 (Box::new(Num::Bit1 (Box::new(Num::Bit0 (Box::new(Num::One)))))))))))
  }

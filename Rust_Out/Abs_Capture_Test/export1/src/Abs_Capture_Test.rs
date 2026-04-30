use crate::HOL::*;
use crate::Num::*;
use crate::Int::*;

pub fn closure_1 
  () -> Int
     {
    {
      let y = one_int();
      let f = (move |x: Int| {
                plus_int(x.clone(), y.clone())
              });
      f (Int::Pos (Num::Bit0 (Box::new(Num::One))))
    }
  }

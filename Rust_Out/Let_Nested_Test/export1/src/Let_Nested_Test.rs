use crate::HOL::*;
use crate::Int::*;

pub fn own 
  (x: Int) -> Int
     {
    {
      let y = one_int();
      let z = plus_int(y.clone(), one_int());
      let _ = plus_int(z.clone(), one_int());
      z.clone()
    }
  }

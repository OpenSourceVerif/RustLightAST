use crate::HOL::*;
use crate::Int::*;

pub fn add1 
  (x: Int) -> Int
     {
    {
      let z = one_int();
      plus_int(z.clone(), z.clone())
    }
  }

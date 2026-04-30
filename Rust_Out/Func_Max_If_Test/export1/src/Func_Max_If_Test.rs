use crate::HOL::*;
use crate::Int::*;

pub fn max 
  (a: Int, b: Int) -> Int
     {
    match less_int(b.clone(), a.clone()) {
      true => a.clone(), 
      false => b.clone()
    }
  }

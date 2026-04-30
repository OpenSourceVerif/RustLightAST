use crate::Product_Type::*;
use crate::Int::*;

pub fn max2 
  (a: Int, b: Int) -> Int
     {
    match less_int(b.clone(), a.clone()) {
      true => a.clone(), 
      false => b.clone()
    }
  }

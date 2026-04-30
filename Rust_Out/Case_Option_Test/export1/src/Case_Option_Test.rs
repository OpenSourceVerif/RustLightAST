use crate::Option::*;
use crate::Int::*;

pub fn get_or_zero 
  (x: Option<Int>) -> Int
     {
    match x.clone() {
      Option::None => Int::ZeroInt, 
      Option::Some (n) => n.clone()
    }
  }

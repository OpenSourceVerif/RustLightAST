use crate::Product_Type::*;
use crate::Int::*;

pub fn int_pair 
  (x: Int) -> Prod<Int, Int>
     {
    match x{x => Prod::Pair (x.clone(), x.clone())}
  }

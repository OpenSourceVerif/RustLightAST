use crate::Int::*;

pub fn add3 
  (x: Int, y: Int, z: Int) -> Int
     {
    match (x, y, z){(x, y, z) => plus_int(plus_int(x.clone(), y.clone()), z.clone())}
  }

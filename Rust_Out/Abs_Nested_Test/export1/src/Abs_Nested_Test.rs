use crate::Int::*;

pub fn add_n_2 
  (n: Int) -> impl Fn(Int, Int) -> Int
     {
    match n{n => (move |x: Int, y: Int| {
                   plus_int(plus_int(x.clone(), y.clone()), n.clone())
                 })}
  }

use crate::Int::*;

pub fn add_n 
  (n: Int) -> impl Fn(Int) -> Int
     {
    match n{n => (move |x: Int| {
                   plus_int(x.clone(), n.clone())
                 })}
  }

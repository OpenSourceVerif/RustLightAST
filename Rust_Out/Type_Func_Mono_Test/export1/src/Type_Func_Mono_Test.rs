use crate::Int::*;

pub fn apply_twice2  (f: impl Fn(Int) -> Int + Clone, x: Int) -> Int
                        {
                       f (f (x.clone()))
                     }

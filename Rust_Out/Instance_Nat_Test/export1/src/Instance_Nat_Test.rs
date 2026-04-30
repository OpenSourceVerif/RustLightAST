use crate::Nat::*;

pub fn inc_nat 
  (n: Nat) -> Nat
     {
    match n{n => plus_nat(n.clone(), one_nat())}
  }

pub fn add2  (x: Nat) -> Nat
                {
               match x{x => inc_nat(x.clone())}
             }

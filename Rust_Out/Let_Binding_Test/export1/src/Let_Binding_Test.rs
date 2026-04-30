use crate::HOL::*;
use crate::Num::*;
use crate::Nat::*;

pub fn test_let 
  (n: Nat) -> Nat
     {
    {
      let x = plus_nat(n.clone(), one_nat());
      times_nat(x.clone(), nat_of_num(Num::Bit0 (Box::new(Num::One))))
    }
  }

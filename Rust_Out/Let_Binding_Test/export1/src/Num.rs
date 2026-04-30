use crate::HOL::*;
use crate::Nat::*;

#[derive(Clone)]
pub enum Num { 
  One, 
  Bit0 (Box<Num>), 
  Bit1 (Box<Num>)
}

pub fn nat_of_num 
  (x0: Num) -> Nat
     {
    match x0 {
      Num::Bit1 (box n) => {
        let m = nat_of_num(n.clone());
        Nat::Suc (Box::new(plus_nat(m.clone(), m.clone())))
      }, 
      Num::Bit0 (box n) => {
        let m = nat_of_num(n.clone());
        plus_nat(m.clone(), m.clone())
      }, 
      Num::One => one_nat()
    }
  }

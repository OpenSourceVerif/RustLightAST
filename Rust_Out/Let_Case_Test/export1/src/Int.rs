use crate::Num::*;

#[derive(Clone)]
pub enum Int { 
  ZeroInt, 
  Pos (Num), 
  Neg (Num)
}

pub fn dup 
  (x0: Int) -> Int
     {
    match x0 {
      Int::Neg (n) => Int::Neg (Num::Bit0 (Box::new(n.clone()))), 
      Int::Pos (n) => Int::Pos (Num::Bit0 (Box::new(n.clone()))), 
      Int::ZeroInt => Int::ZeroInt
    }
  }

pub fn uminus_int 
  (x0: Int) -> Int
     {
    match x0 {
      Int::Neg (m) => Int::Pos (m.clone()), 
      Int::Pos (m) => Int::Neg (m.clone()), 
      Int::ZeroInt => Int::ZeroInt
    }
  }

pub fn one_int 
  () -> Int
     {
    Int::Pos (Num::One)
  }

pub fn sub 
  (x0: Num, x1: Num) -> Int
     {
    match (x0, x1) {
      (Num::Bit0 (box m), Num::Bit1 (box n)) => minus_int(dup(sub(m.clone(), n.clone())), one_int()), 
      (Num::Bit1 (box m), Num::Bit0 (box n)) => plus_int(dup(sub(m.clone(), n.clone())), one_int()), 
      (Num::Bit1 (box m), Num::Bit1 (box n)) => dup(sub(m.clone(), n.clone())), 
      (Num::Bit0 (box m), Num::Bit0 (box n)) => dup(sub(m.clone(), n.clone())), 
      (Num::One, Num::Bit1 (box n)) => Int::Neg (Num::Bit0 (Box::new(n.clone()))), 
      (Num::One, Num::Bit0 (box n)) => Int::Neg (bitm(n.clone())), 
      (Num::Bit1 (box m), Num::One) => Int::Pos (Num::Bit0 (Box::new(m.clone()))), 
      (Num::Bit0 (box m), Num::One) => Int::Pos (bitm(m.clone())), 
      (Num::One, Num::One) => Int::ZeroInt
    }
  }

pub fn plus_int 
  (k: Int, l: Int) -> Int
     {
    match (k, l) {
      (Int::Neg (m), Int::Neg (n)) => Int::Neg (plus_num(m.clone(), n.clone())), 
      (Int::Neg (m), Int::Pos (n)) => sub(n.clone(), m.clone()), 
      (Int::Pos (m), Int::Neg (n)) => sub(m.clone(), n.clone()), 
      (Int::Pos (m), Int::Pos (n)) => Int::Pos (plus_num(m.clone(), n.clone())), 
      (Int::ZeroInt, l) => l.clone(), 
      (k, Int::ZeroInt) => k.clone()
    }
  }

pub fn minus_int 
  (k: Int, l: Int) -> Int
     {
    match (k, l) {
      (Int::Neg (m), Int::Neg (n)) => sub(n.clone(), m.clone()), 
      (Int::Neg (m), Int::Pos (n)) => Int::Neg (plus_num(m.clone(), n.clone())), 
      (Int::Pos (m), Int::Neg (n)) => Int::Pos (plus_num(m.clone(), n.clone())), 
      (Int::Pos (m), Int::Pos (n)) => sub(m.clone(), n.clone()), 
      (Int::ZeroInt, l) => uminus_int(l.clone()), 
      (k, Int::ZeroInt) => k.clone()
    }
  }

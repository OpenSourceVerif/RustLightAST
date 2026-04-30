#[derive(Clone)]
pub enum Num { 
  One, 
  Bit0 (Box<Num>), 
  Bit1 (Box<Num>)
}

pub fn less_num 
  (m: Num, x1: Num) -> bool
     {
    match (m, x1) {
      (Num::Bit1 (box m), Num::Bit0 (box n)) => less_num(m.clone(), n.clone()), 
      (Num::Bit1 (box m), Num::Bit1 (box n)) => less_num(m.clone(), n.clone()), 
      (Num::Bit0 (box m), Num::Bit1 (box n)) => less_eq_num(m.clone(), n.clone()), 
      (Num::Bit0 (box m), Num::Bit0 (box n)) => less_num(m.clone(), n.clone()), 
      (Num::One, Num::Bit1 (box n)) => true, 
      (Num::One, Num::Bit0 (box n)) => true, 
      (m, Num::One) => false
    }
  }

pub fn less_eq_num 
  (x0: Num, n: Num) -> bool
     {
    match (x0, n) {
      (Num::Bit1 (box m), Num::Bit0 (box n)) => less_num(m.clone(), n.clone()), 
      (Num::Bit1 (box m), Num::Bit1 (box n)) => less_eq_num(m.clone(), n.clone()), 
      (Num::Bit0 (box m), Num::Bit1 (box n)) => less_eq_num(m.clone(), n.clone()), 
      (Num::Bit0 (box m), Num::Bit0 (box n)) => less_eq_num(m.clone(), n.clone()), 
      (Num::Bit1 (box m), Num::One) => false, 
      (Num::Bit0 (box m), Num::One) => false, 
      (Num::One, n) => true
    }
  }

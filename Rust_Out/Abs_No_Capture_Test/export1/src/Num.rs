#[derive(Clone)]
pub enum Num { 
  One, 
  Bit0 (Box<Num>), 
  Bit1 (Box<Num>)
}

pub fn bitm 
  (x0: Num) -> Num
     {
    match x0 {
      Num::One => Num::One, 
      Num::Bit0 (box n) => Num::Bit1 (Box::new(bitm(n.clone()))), 
      Num::Bit1 (box n) => Num::Bit1 (Box::new(Num::Bit0 (Box::new(n.clone()))))
    }
  }

pub fn plus_num 
  (x0: Num, x1: Num) -> Num
     {
    match (x0, x1) {
      (Num::Bit1 (box m), Num::Bit1 (box n)) => Num::Bit0 (Box::new(plus_num(plus_num(m.clone(), n.clone()), Num::One))), 
      (Num::Bit1 (box m), Num::Bit0 (box n)) => Num::Bit1 (Box::new(plus_num(m.clone(), n.clone()))), 
      (Num::Bit1 (box m), Num::One) => Num::Bit0 (Box::new(plus_num(m.clone(), Num::One))), 
      (Num::Bit0 (box m), Num::Bit1 (box n)) => Num::Bit1 (Box::new(plus_num(m.clone(), n.clone()))), 
      (Num::Bit0 (box m), Num::Bit0 (box n)) => Num::Bit0 (Box::new(plus_num(m.clone(), n.clone()))), 
      (Num::Bit0 (box m), Num::One) => Num::Bit1 (Box::new(m.clone())), 
      (Num::One, Num::Bit1 (box n)) => Num::Bit0 (Box::new(plus_num(n.clone(), Num::One))), 
      (Num::One, Num::Bit0 (box n)) => Num::Bit1 (Box::new(n.clone())), 
      (Num::One, Num::One) => Num::Bit0 (Box::new(Num::One))
    }
  }

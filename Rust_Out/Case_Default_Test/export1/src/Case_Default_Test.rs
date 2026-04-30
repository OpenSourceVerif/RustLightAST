use crate::Nat::*;

#[derive(Clone)]
pub enum Color { 
  Red, 
  Green, 
  Blue, 
  Other (Nat)
}

pub fn is_primary 
  (x0: Color) -> bool
     {
    match x0 {
      Color::Red => true, 
      Color::Green => true, 
      Color::Blue => true, 
      Color::Other (v) => false
    }
  }

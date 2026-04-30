use crate::Int::*;

#[derive(Clone)]
pub enum Option { 
  None, 
  Some (Int), 
  Rec (Box<Option>)
}

pub fn get 
  (x0: Option) -> Int
     {
    match x0 {
      Option::Some (x) => x.clone(), 
      Option::None => Int::ZeroInt, 
      Option::Rec (box op) => get(op.clone())
    }
  }

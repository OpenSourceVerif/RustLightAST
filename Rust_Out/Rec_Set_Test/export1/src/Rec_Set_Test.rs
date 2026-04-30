use crate::Int::*;

#[derive(Clone)]
pub enum Option { 
  None, 
  Some (Int), 
  Rec (Box<Option>)
}

pub fn set 
  (xa0: Option, x: Int) -> Option
     {
    match (xa0, x) {
      (Option::Some (uu), x) => Option::Some (x.clone()), 
      (Option::None, x) => Option::Some (x.clone()), 
      (Option::Rec (box op), x) => Option::Rec (Box::new(set(op.clone(), x.clone())))
    }
  }

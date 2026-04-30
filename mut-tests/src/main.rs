#[derive(Clone)]
pub enum Num { 
    One, 
    Bit0(Box<Num>), 
    Bit1(Box<Num>),
}

#[derive(Clone)]
pub enum Int { 
    ZeroInt, 
    Pos(Num), 
    Neg(Num),
}

#[derive(Clone)]
pub enum Option { 
    None, 
    Some(Int), 
    Rec(Box<Option>),
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


fn main() {}
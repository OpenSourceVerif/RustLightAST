use crate::Int::*;

#[derive(Clone)]
pub enum Aoption { 
  Nonea, 
  Somea (Int), 
  MutualReca (Box<Boption>)
}
#[derive(Clone)]
pub enum Boption { 
  Noneb, 
  Someb (Int), 
  MutualRecb (Box<Aoption>)
}

pub fn mugeta 
  (x0: Aoption) -> Int
     {
    match x0 {
      Aoption::Somea (x) => x.clone(), 
      Aoption::Nonea => Int::ZeroInt, 
      Aoption::MutualReca (box bop) => mugetb(bop.clone())
    }
  }

pub fn mugetb 
  (x0: Boption) -> Int
     {
    match x0 {
      Boption::Someb (x) => x.clone(), 
      Boption::Noneb => Int::ZeroInt, 
      Boption::MutualRecb (box aop) => mugeta(aop.clone())
    }
  }

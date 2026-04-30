use crate::Int::*;
use crate::Num::*;
use crate::Product_Type::*;

#[derive(Clone)]
pub enum PointExt<A> { 
  PointExt (Int, Int, A)
}

pub fn pt1 
  () -> PointExt<Unit>
     {
    PointExt::PointExt (Int::Pos (Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::Bit0 (Box::new(Num::Bit0 (Box::new(Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::One))))))))))))))))))), Int::Pos (Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::Bit1 (Box::new(Num::Bit0 (Box::new(Num::One))))))))), Unit::Unity)
  }

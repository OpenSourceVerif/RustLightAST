use crate::Num::*;
#[derive(Clone)]
pub enum Int {
    ZeroInt,
    Pos(Num),
    Neg(Num),
}


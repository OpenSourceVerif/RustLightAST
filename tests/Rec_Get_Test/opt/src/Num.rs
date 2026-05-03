#[derive(Clone)]
pub enum Num {
    One,
    Bit0(Box<Num>),
    Bit1(Box<Num>),
}


#[derive(Clone)]
pub enum Nat { 
  ZeroNat, 
  Suc (Box<Nat>)
}

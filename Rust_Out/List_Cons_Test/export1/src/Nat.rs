#[derive(Clone)]
pub enum Nat { 
  ZeroNat, 
  Suc (Box<Nat>)
}

pub fn one_nat 
  () -> Nat
     {
    Nat::Suc (Box::new(Nat::ZeroNat))
  }

pub fn plus_nat 
  (x0: Nat, n: Nat) -> Nat
     {
    match (x0, n) {
      (Nat::Suc (box m), n) => plus_nat(m.clone(), Nat::Suc (Box::new(n.clone()))), 
      (Nat::ZeroNat, n) => n.clone()
    }
  }

pub trait Add2 {
  fn add2 (x0: Self) -> Self;
}

pub trait Add1 {
  fn add1 (x0: Self) -> Self;
}

pub trait Add1Add2 : Add1 + Add2 {
}

pub fn add12 <A>
  (x: A) -> A
    where
      A : Clone + Add1Add2
     {
    A::add2(A::add1(x.clone()))
  }

pub trait Inc {
  fn inc (x0: Self) -> Self;
}

pub fn add1 <A>
  (x: A) -> A
    where
      A : Clone + Inc
     {
    match x{x => A::inc(x.clone())}
  }

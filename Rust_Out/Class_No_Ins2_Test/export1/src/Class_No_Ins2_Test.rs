pub trait Plus {
  fn plus (x0: Self, x1: Self) -> Self;
}

pub fn add_pair <A>
  (x: A, y: A) -> A
    where
      A : Clone + Plus
     {
    match (x, y){(x, y) => A::plus(x.clone(), y.clone())}
  }

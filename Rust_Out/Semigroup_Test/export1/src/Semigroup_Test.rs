pub trait Semigroup {
  fn plus (x0: Self, x1: Self) -> Self;
}

pub trait Monoid : Semigroup {
  fn zero () -> Self;
}

pub fn cls <A>
  (x: A, y: A) -> A
    where
      A : Clone + Monoid
     {
    match (x, y){(x, y) => A::plus(x.clone(), y.clone())}
  }

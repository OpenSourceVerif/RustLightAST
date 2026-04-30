pub fn apply_twice <A>
  (f: impl Fn(A) -> A + Clone, x: A) -> A
    where
      A : Clone
     {
    f (f (x.clone()))
  }

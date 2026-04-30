#[derive(Clone)]
pub enum List<A> { 
  Nil, 
  Cons (A, Box<List<A>>)
}

pub fn fold <A, B>
  (f: impl Fn(A, B) -> B + Clone, x1: List<A>, s: B) -> B
    where
      A : Clone, 
      B : Clone
     {
    match (f, x1, s) {
      (f, List::Cons (x, box xs), s) => fold(f.clone(), xs.clone(), f (x.clone(), s.clone())), 
      (f, List::Nil, s) => s.clone()
    }
  }

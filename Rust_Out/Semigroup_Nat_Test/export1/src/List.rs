#[derive(Clone)]
pub enum List<A> { 
  Nil, 
  Cons (A, Box<List<A>>)
}

pub fn mapa <A, B>
  (f: impl Fn(A) -> B + Clone, x1: List<A>) -> List<B>
    where
      A : Clone, 
      B : Clone
     {
    match (f, x1) {
      (f, List::Nil) => List::Nil, 
      (f, List::Cons (x21, box x22)) => List::Cons (f (x21.clone()), Box::new(mapa(f.clone(), x22.clone())))
    }
  }

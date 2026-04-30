#[derive(Clone)]
pub enum List<A> { 
  Nil, 
  Cons (A, Box<List<A>>)
}

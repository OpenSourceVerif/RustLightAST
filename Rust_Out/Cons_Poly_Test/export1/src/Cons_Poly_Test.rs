use crate::Groups::*;

pub fn zero <A> () -> A
                  where
                    A : Clone + Zero
                   {
                  A::zero()
                }

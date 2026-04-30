use crate::Product_Type::*;

pub fn neg  (b: bool) -> bool
               {
              match b.clone() {
                true => false, 
                false => true
              }
            }

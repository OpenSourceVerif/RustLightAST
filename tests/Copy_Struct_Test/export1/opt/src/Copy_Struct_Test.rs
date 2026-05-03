#[derive(Clone)]
pub enum FlagPair {
    FlagPair(bool, bool),
}

pub fn get_left(x0: FlagPair) -> bool {
    match x0 {
        FlagPair::FlagPair(x, y) => {
            x.clone()
        },
    }
}

pub fn get_right(x0: FlagPair) -> bool {
    match x0 {
        FlagPair::FlagPair(x, y) => {
            y.clone()
        },
    }
}

pub fn swap_flag_pair(x0: FlagPair) -> FlagPair {
    match x0 {
        FlagPair::FlagPair(x, y) => {
            FlagPair::FlagPair(y.clone(), x.clone())
        },
    }
}


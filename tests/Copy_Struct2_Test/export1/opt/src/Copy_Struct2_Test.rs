#[derive(Clone, Copy)]
pub enum Color {
    Red,
    Green,
    Blue,
}

#[derive(Clone, Copy)]
pub enum Pixel {
    Pixel(Color, Color, Color),
}

pub fn is_red(x0: Color) -> bool {
    match x0 {
        Color::Red => {
            true
        },
        Color::Green => {
            false
        },
        Color::Blue => {
            false
        },
    }
}



pub fn rotate_pixel(x0: Pixel) -> Pixel {
    match x0 {
        Pixel::Pixel(r, g, b) => {
            Pixel::Pixel(g, b, r)
        },
    }
}

pub fn get_first_color(x0: Pixel) -> Color {
    match x0 {
        Pixel::Pixel(r, g, b) => {
            r
        },
    }
}

pub fn replace_first_color(x0: Pixel, c: Color) -> Pixel {
    match (x0, c) {
        (Pixel::Pixel(r, g, b), c) => {
            Pixel::Pixel(c, g, b)
        },
    }
}


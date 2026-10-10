pub enum Shape {
    Circle(u32),
    Empty,
}

pub trait Area {
    fn area(&self) -> u32;
}

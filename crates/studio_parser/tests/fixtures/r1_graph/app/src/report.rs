// `std::fmt` and `std::mem::swap` are outside the workspace; the name match linked fmt.rs (same stem, same fn name).
use std::collections::HashMap;
use std::fmt;

pub fn report(_out: &mut fmt::Formatter, mut a: u8, mut b: u8) -> HashMap<u8, u8> {
    std::mem::swap(&mut a, &mut b);
    HashMap::new()
}

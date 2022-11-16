mod controller;
pub use controller::*;


pub trait Command: Into<u8> {}

pub trait Argument: Into<u8> {}
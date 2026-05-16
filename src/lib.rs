#![allow(dead_code)]
#![allow(unused_imports)]
#![allow(unused_variables)]
#![allow(unused_assignments)]
#![allow(clippy::type_complexity)]
#![allow(clippy::from_over_into)]
#![allow(clippy::module_inception)]

#[macro_use]
extern crate anyhow;
#[macro_use]
extern crate log;

pub mod dicts;
pub mod lang;
pub mod parser;
pub mod render;

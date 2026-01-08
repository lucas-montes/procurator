// Parse a repository and initialize it's configurations

use std::env;

use crate::autonix::Parser;

pub fn init() {
    println!("Running autonix");
     let path = env::current_dir().expect("Failed to get current directory");
    Parser::new(path);
}

// The repo is where the logic to create the environment and parse dependencies for individual
// repositories, either regular repositories or monorepos with multiple services inside it. A repo
// is equivalent to a single service that will output a single flake. They are different from
// projects which are a collection of repos.

mod parser;
mod scan;
mod analysis;
mod flake;

pub use parser::Parser;

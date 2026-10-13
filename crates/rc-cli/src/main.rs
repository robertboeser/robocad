//! Headless command-line tool for robocad.

use clap::Parser;

#[derive(Parser)]
#[command(name = "robocad-cli", version, about = "Headless robocad tools")]
struct Cli {}

fn main() {
    let _cli = Cli::parse();
}

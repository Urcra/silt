//! A stand-in for `ssh` in the tests of plugins on the `ssh` capability:
//! prints the arguments it was given, one a line, then what it read on
//! stdin, and exits 0.

use std::io::Read as _;

fn main() {
    for arg in std::env::args().skip(1) {
        println!("{arg}");
    }
    let mut stdin = String::new();
    let _ = std::io::stdin().read_to_string(&mut stdin);
    print!("stdin: {stdin}");
}

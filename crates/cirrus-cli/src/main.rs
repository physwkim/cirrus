//! `cirrus` CLI — minimal stub that prints version and exits.
//!
//! Real REPL is M6 work.

fn main() {
    println!("cirrus {}", env!("CARGO_PKG_VERSION"));
    println!("Rust DAQ RunEngine. (REPL not yet implemented.)");
}

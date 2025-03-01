//! This is the Hello World example from the README.

#[test]
fn main() {
    // A bijective map between letters of the English alphabet and their positions.
    let mut alphabet = bimap::BiMap::<char, u8>::new();

    alphabet.insert('A', 1);
    // some letters omitted for brevity
    alphabet.insert('Z', 26);

    println!("A is at position {}", alphabet.get_by_left(&'A').unwrap());
    println!("{} is at position 26", alphabet.get_by_right(&26).unwrap());
}

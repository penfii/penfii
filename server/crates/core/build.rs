// The migrations are compiled into the binary by `sqlx::migrate!`; cargo does not know that the
// directory is an input, so tell it.
fn main() {
    println!("cargo:rerun-if-changed=../../migrations");
}

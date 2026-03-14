fn main() {
    // Print rerun instruction so Cargo re-runs this script when configs change.
    println!("cargo:rerun-if-changed=configs/");
    println!("cargo:rerun-if-changed=build.rs");
}

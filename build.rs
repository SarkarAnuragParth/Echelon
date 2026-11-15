fn main() {
    // include_dir!/include_str! don't reliably trigger a rebuild when only
    // the embedded content changes under this toolchain — confirmed the
    // hard way once already. These make Cargo watch both trees explicitly.
    println!("cargo:rerun-if-changed=scaffold-template");
    println!("cargo:rerun-if-changed=prompts");
}

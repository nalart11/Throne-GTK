fn main() {
    println!("cargo:rerun-if-changed=proto/libcore.proto");
    prost_build::compile_protos(&["proto/libcore.proto"], &["proto"])
        .expect("compiling libcore.proto");
}

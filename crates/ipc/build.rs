fn main() {
    println!("cargo:rerun-if-changed=proto/libcore.proto");
    prost_build::compile_protos(&["proto/libcore.proto"], &["proto"])
        .expect("compiling libcore.proto");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("src/service_macos.m")
            .flag("-fobjc-arc")
            .compile("throne_service_macos");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=ServiceManagement");
    }
}

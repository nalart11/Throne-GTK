fn main() {
    println!("cargo:rerun-if-changed=proto/libcore.proto");
    prost_build::compile_protos(&["proto/libcore.proto"], &["proto"])
        .expect("compiling libcore.proto");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        println!("cargo:rerun-if-changed=src/service_macos.m");
        println!("cargo:rerun-if-changed=../../core/helpertrust/security.h");
        println!("cargo:rerun-if-changed=../../core/helpertrust/acl_policy.h");
        cc::Build::new()
            .file("src/service_macos.m")
            .flag("-fobjc-arc")
            .compile("throne_service_macos");
        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=ServiceManagement");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
    }
}

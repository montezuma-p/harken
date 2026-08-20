use std::env;
use std::path::Path;

fn common_build(build: &mut cc::Build, whisper_dir: &Path, target: &str) {
    build
        .warnings(false)
        .flag_if_supported("-O3")
        .include(whisper_dir)
        .include(whisper_dir.join("include"))
        .include(whisper_dir.join("ggml/include"))
        .include(whisper_dir.join("ggml/src"))
        .include(whisper_dir.join("ggml/src/ggml-cpu"))
        .define("GGML_USE_CPU", None)
        .define("_XOPEN_SOURCE", Some("600"));

    if target.contains("linux") || target.contains("android") {
        build.define("_GNU_SOURCE", None);
    }

    if target.contains("apple") {
        build.define("_DARWIN_C_SOURCE", None);
    }

    if target.contains("windows") {
        build.define("_CRT_SECURE_NO_WARNINGS", None);
    }
}

fn cpp_stdlib(target: &str) -> Option<&'static str> {
    if target.contains("msvc") {
        None
    } else if target.contains("apple")
        || target.contains("freebsd")
        || target.contains("openbsd")
        || target.contains("netbsd")
    {
        Some("c++")
    } else {
        Some("stdc++")
    }
}

fn main() {
    let whisper_dir = Path::new("vendor/whisper.cpp");

    if !whisper_dir.join("include/whisper.h").exists() {
        panic!("vendor/whisper.cpp not found. Run: git submodule update --init --recursive");
    }

    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/include/whisper.h");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/src/whisper.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-alloc.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-backend.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-backend-reg.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-opt.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-threading.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-quants.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/gguf.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/ggml-cpu.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/ggml-cpu.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/binary-ops.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/hbm.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/ops.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/quants.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/repack.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/traits.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/unary-ops.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/vec.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/arch/x86/quants.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/arch/x86/repack.cpp");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/arch/arm/quants.c");
    println!("cargo:rerun-if-changed=vendor/whisper.cpp/ggml/src/ggml-cpu/arch/arm/repack.cpp");

    let target = env::var("TARGET").expect("TARGET is not set");

    let mut c_build = cc::Build::new();
    common_build(&mut c_build, whisper_dir, &target);
    c_build
        .file(whisper_dir.join("ggml/src/ggml.c"))
        .file(whisper_dir.join("ggml/src/ggml-alloc.c"))
        .file(whisper_dir.join("ggml/src/ggml-quants.c"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/ggml-cpu.c"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/quants.c"));

    let mut cpp_build = cc::Build::new();
    common_build(&mut cpp_build, whisper_dir, &target);
    cpp_build
        .cpp(true)
        .flag_if_supported("-std=c++17")
        .file(whisper_dir.join("src/whisper.cpp"))
        .file(whisper_dir.join("ggml/src/ggml.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-backend.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-backend-reg.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-opt.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-threading.cpp"))
        .file(whisper_dir.join("ggml/src/gguf.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/binary-ops.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/ggml-cpu.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/hbm.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/ops.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/repack.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/traits.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/unary-ops.cpp"))
        .file(whisper_dir.join("ggml/src/ggml-cpu/vec.cpp"));

    if target.contains("x86_64") || target.contains("i686") {
        cpp_build
            .file(whisper_dir.join("ggml/src/ggml-cpu/arch/x86/repack.cpp"))
            .file(whisper_dir.join("ggml/src/ggml-cpu/amx/amx.cpp"))
            .file(whisper_dir.join("ggml/src/ggml-cpu/amx/mmq.cpp"));
        c_build.file(whisper_dir.join("ggml/src/ggml-cpu/arch/x86/quants.c"));
    }

    if target.contains("aarch64") || target.contains("arm") {
        cpp_build.file(whisper_dir.join("ggml/src/ggml-cpu/arch/arm/repack.cpp"));
        c_build.file(whisper_dir.join("ggml/src/ggml-cpu/arch/arm/quants.c"));
    }

    c_build.compile("whisper_c");
    cpp_build.compile("whisper_cpp");

    if let Some(cpp_stdlib) = cpp_stdlib(&target) {
        println!("cargo:rustc-link-lib={cpp_stdlib}");
    }

    if target.contains("apple") {
        println!("cargo:rustc-link-lib=framework=Accelerate");
    }
}

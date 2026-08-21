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
        // CMAKE_BUILD_TYPE=Release implies -DNDEBUG, which cc-rs does not add.
        // Without it every assert() in ggml's operator loops stays compiled in.
        .define("NDEBUG", None)
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

    arch_flags(build, target);
}

/// ggml's CPU kernels are selected at *compile* time: `arch/x86/quants.c` and
/// `simd-mappings.h` gate on `__AVX2__`/`__F16C__`, so a build with no `-m`
/// flags silently falls back to scalar code — same binary, several times the
/// wall clock. whisper.cpp's CMake avoids that by defaulting to
/// `-march=native`, which is right for a machine-local build and wrong for a
/// distributed one (the release binary would inherit the CI runner's ISA).
///
/// So: a fixed, portable floor by default — AVX2/FMA/F16C, i.e. Haswell (2013)
/// and newer, which is narrower than what `-march=native` on a CI runner
/// produces — and `HARKEN_NATIVE=1` for anyone who wants their own CPU's full
/// instruction set out of a source build.
fn arch_flags(build: &mut cc::Build, target: &str) {
    println!("cargo:rerun-if-env-changed=HARKEN_NATIVE");
    let native = env::var("HARKEN_NATIVE").is_ok_and(|v| v != "0");

    if native && !target.contains("msvc") {
        // clang on arm64 wants -mcpu=native; -march=native is the x86 spelling
        // and GCC rejects -mcpu there, so pick by target rather than probing.
        if target.contains("aarch64") || target.contains("arm") {
            build.flag_if_supported("-mcpu=native");
        } else {
            build.flag_if_supported("-march=native");
        }
        return;
    }

    if !(target.contains("x86_64") || target.contains("i686")) {
        // aarch64/arm: NEON is baseline in armv8 and ggml uses it unconditionally.
        return;
    }

    // Mirrors the ARCH_DEFINITIONS the ggml CMake sets alongside these flags,
    // so ggml_cpu_has_avx2() and friends keep reporting the truth.
    build
        .define("GGML_AVX", None)
        .define("GGML_AVX2", None)
        .define("GGML_FMA", None)
        .define("GGML_F16C", None);

    if target.contains("msvc") {
        build.flag("/arch:AVX2");
    } else {
        build
            .flag_if_supported("-mavx")
            .flag_if_supported("-mavx2")
            .flag_if_supported("-mfma")
            .flag_if_supported("-mf16c");
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
        // Must be std(), not flag("-std=c++17"): MSVC spells it /std:c++17 and
        // silently ignored the unix form, leaving ggml-backend-reg.cpp to compile
        // as C++14 with no std::filesystem. cc picks the right spelling.
        .std("c++17")
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

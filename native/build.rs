use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

fn main() {
    println!("cargo:rerun-if-env-changed=HERDR_RIG_JS_RUNTIME");
    for path in [
        "build.rs",
        "rig/RigBridge.h",
        "rig/RigBridge.swift",
        "rig/RigMotion.swift",
        "rig/RigNativeHost.swift",
        "rig/RigModel.swift",
        "rig/RigDecoder.swift",
        "rig/RigDecodeWorker.swift",
        "rig/RigDecodeWorker.entitlements.plist",
        "rig/RigDeformation.swift",
        "rig/RigHitTesting.swift",
        "rig/RigMetalRenderer.swift",
        "rig/RigMetalShaders.swift",
        "rig/limits.json",
        "rig/build-decoder.mjs",
        "rig/build-limits.mjs",
        "rig/decoder.ts",
        "rig/override-validation.ts",
        "rig/NOTICE.txt",
        "../scripts/build-rig-native.mjs",
        "../web/rig/vendor",
        "../web/rig/package.json",
        "../web/rig/package-lock.json",
    ] {
        println!("cargo:rerun-if-changed={path}");
    }
    if env::var("CARGO_CFG_TARGET_OS").ok().as_deref() != Some("macos") {
        return;
    }
    let manifest_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
    let project_root = manifest_dir.parent().expect("native manifest parent");
    let profile = env::var("PROFILE").expect("PROFILE");
    let runtime = env::var_os("HERDR_RIG_JS_RUNTIME").unwrap_or_else(|| OsString::from("bun"));
    let status = Command::new(&runtime)
        .arg(project_root.join("scripts/build-rig-native.mjs"))
        .current_dir(project_root)
        .env("HERDR_RIG_BUILD_PROFILE", &profile)
        .env_remove("RIG_PROBE_FAULTS")
        .stdin(Stdio::null())
        .status()
        .unwrap_or_else(|error| {
            panic!(
                "cannot execute {} for native rig build: {error}",
                Path::new(&runtime).display()
            )
        });
    assert!(status.success(), "native rig build failed with {status}");
    let output = manifest_dir.join("target/rig-native").join(profile);
    println!(
        "cargo:rustc-env=HERDR_RIG_LIMITS_RS={}",
        output.join("Resources/rig/RigLimits.rs").display()
    );
    let dylib = output.join("libherdr_rig.dylib");
    let worker = output.join("rig-decode-worker");
    println!("cargo:rustc-link-search=native={}", output.display());
    println!("cargo:rustc-link-lib=dylib=herdr_rig");
    println!("cargo:rustc-link-arg=-Wl,-rpath,{}", output.display());
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    println!("cargo:rustc-env=HERDR_RIG_NATIVE_DYLIB={}", dylib.display());
    println!(
        "cargo:rustc-env=HERDR_RIG_DECODE_WORKER={}",
        worker.display()
    );
}

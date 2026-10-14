fn main() {
    // Le backend Metal de whisper.cpp utilise `@available`, qui appelle
    // `__isPlatformVersionAtLeast` (runtime clang). Quand une version minimale de macOS
    // est imposée (bundle Tauri), rustc ne lie plus ce runtime : on l'ajoute nous-mêmes.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && let Ok(out) = std::process::Command::new("clang").arg("--print-runtime-dir").output()
    {
        let dir = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !dir.is_empty() && std::path::Path::new(&dir).join("libclang_rt.osx.a").exists() {
            println!("cargo:rustc-link-search=native={dir}");
            println!("cargo:rustc-link-lib=static=clang_rt.osx");
        }
    }
    // whisper-rs cherche libcudart dans /usr/local/cuda (codé en dur). Sur une machine où ce lien
    // pointe vers une autre version que celle du compilateur (ex. CUDA 13 installé à côté de 12.9),
    // MURMURE_CUDA_LIB_DIR impose le bon dossier.
    println!("cargo:rerun-if-env-changed=MURMURE_CUDA_LIB_DIR");
    if let Ok(dir) = std::env::var("MURMURE_CUDA_LIB_DIR") {
        println!("cargo:rustc-link-search=native={dir}");
        println!("cargo:rustc-link-search=native={dir}/stubs");
    }
    tauri_build::build()
}

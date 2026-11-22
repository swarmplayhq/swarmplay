fn main() {
    slint_build::compile_with_config(
        "ui/app.slint",
        slint_build::CompilerConfiguration::new()
            .embed_resources(slint_build::EmbedResourcesKind::EmbedFiles)
            // The translations (src/i18n.rs): one .po per language, shared with Rust's tr!. No
            // per-component context, so a text is translated once wherever it appears.
            .with_bundled_translations("i18n")
            .with_default_translation_context(slint_build::DefaultTranslationContext::None),
    )
    .expect("Slint UI failed to compile");

    // The PS4 RuTracker snapshot is bundled when it has been collected; until then the
    // launcher simply has no PS4 catalog (an empty file here).
    let source = std::path::Path::new("assets/rutracker/ps4-topics.json");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("ps4-topics.json");
    std::fs::write(&out, std::fs::read(source).unwrap_or_default()).expect("could not stage the PS4 catalog");
    println!("cargo:rerun-if-changed=assets/rutracker/ps4-topics.json");

    // The PKG extractor's write guard (src/pkgguard.c), embedded and preloaded at run time.
    let guard = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("pkgguard.so");
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".into());
    let status = std::process::Command::new(&cc)
        .args(["-shared", "-fPIC", "-O2", "-Wall", "-Wextra", "-Werror", "-o"])
        .arg(&guard)
        .arg("src/pkgguard.c")
        .args(["-ldl", "-lpthread"])
        .status()
        .unwrap_or_else(|e| panic!("could not run the C compiler ({cc}): {e}"));
    assert!(status.success(), "building src/pkgguard.c failed");
    println!("cargo:rerun-if-changed=src/pkgguard.c");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=i18n");

    // Rust's texts (tr!, src/i18n.rs) read each language as a .mo made here from the same .po
    // the window uses. (tr's own .po reader ignores in-memory content in rspolib 0.1.2.)
    for lang in ["pt_BR"] {
        use rspolib::SaveAsMOFile;
        let po = format!("i18n/{lang}/LC_MESSAGES/azrak.po");
        let mut file = rspolib::pofile(po.as_str()).unwrap_or_else(|e| panic!("{po}: {e}"));
        // "nplurals=2; plural=(n > 1);" → "nplurals=2;plural=(n > 1);": tr 0.1.11 doesn't trim the
        // parts, so " plural" wouldn't be found and every language would get English's plurals.
        if let Some(rule) = file.metadata.get_mut("Plural-Forms") {
            *rule = rule.split(';').map(str::trim).collect::<Vec<_>>().join(";");
        }
        let mo = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap()).join(format!("{lang}.mo"));
        file.save_as_mofile(mo.to_str().unwrap());
    }
}

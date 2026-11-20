//! Installs the catalog closure from the downloaded `complete.zip` into a
//! temporary directory. Skipped (with a note) when the zip isn't present.

use sf_ldraw::{MeshLibrary, install_from_zip, is_installed};

#[test]
fn install_catalog_closure() {
    let zip = sf_ldraw::default_zip_path();
    if !zip.is_file() {
        eprintln!("NOTE: skipping install test, {} not found", zip.display());
        return;
    }
    let dest = std::env::temp_dir().join(format!("sf-ldraw-install-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dest);
    let report = install_from_zip(&zip, &dest).expect("install");
    let result = std::panic::catch_unwind(|| {
        assert!(report.missing.is_empty(), "missing: {:?}", report.missing);
        assert!(report.files > sf_core::CATALOG.len());
        assert!(is_installed(&dest));
        assert!(dest.join("LDConfig.ldr").is_file() && dest.join("CAreadme.txt").is_file());
        // Subparts and primitives are present (lowercase paths).
        assert!(dest.join("parts/s/3001s01.dat").is_file());
        assert!(dest.join("p/stud.dat").is_file());
        let lib = MeshLibrary::open(&dest).unwrap();
        for d in sf_core::CATALOG {
            assert!(lib.mesh(d.id).is_some_and(|m| m.triangle_count() > 20), "{}", d.id);
        }
        assert!(lib.missing_files().is_empty(), "{:?}", lib.missing_files());
        let colors = lib.ldconfig_colors();
        assert!(colors.iter().any(|(c, n, _)| *c == 4 && n == "Red"), "LDConfig parsed");
    });
    let _ = std::fs::remove_dir_all(&dest);
    if let Err(e) = result {
        std::panic::resume_unwind(e);
    }
}

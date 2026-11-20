//! LDraw support for studforge.
//!
//! * [`install`]: extract the parts the catalog needs from the LDraw
//!   `complete.zip` into a cache directory.
//! * [`parse`]: parse LDraw files and flatten a part (with all subfiles) into
//!   a [`PartMesh`] in LDraw coordinates (LDU, -Y up).
//! * [`MeshLibrary`]: cached meshes by part id, from an LDraw directory or
//!   generated procedurally from the catalog when no library is available.
//!
//! The LDraw parts library is licensed CC BY 4.0 (see `CAreadme.txt` in the
//! installed cache); it is never vendored into this repository.

pub mod install;
pub mod library;
pub mod parse;
pub mod procedural;

pub use install::{InstallReport, default_cache_dir, default_zip_path, ensure_installed, install_from_zip, is_installed};
pub use library::{MeshLibrary, parse_ldconfig};
pub use parse::{MAIN_COLOR, PartMesh, Xform};
pub use procedural::procedural_mesh;

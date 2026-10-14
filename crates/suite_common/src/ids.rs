//! What saved projects know a plugin by. Each plugin's `ids` test compares this with its
//! `ids.txt`, so a renamed or removed id fails the tests (see Versioning in CLAUDE.md).

use nih_plug::prelude::*;
use std::fmt::Write;

/// Bundle name, `NAME`, `CLAP_ID`, `VST3_CLASS_ID`, then every parameter id and persisted key,
/// one per line. `package` is the plugin's entry in `bundler.toml`.
pub fn identity<P: ClapPlugin + Vst3Plugin>(bundler_toml: &str, package: &str) -> String {
    let mut out = String::new();
    writeln!(out, "bundle {}", bundle_name(bundler_toml, package)).unwrap();
    writeln!(out, "name {}", P::NAME).unwrap();
    writeln!(out, "clap {}", P::CLAP_ID).unwrap();
    writeln!(out, "vst3 {}", String::from_utf8_lossy(&P::VST3_CLASS_ID)).unwrap();

    let params = P::default().params();
    let mut ids: Vec<String> = params.param_map().into_iter().map(|(id, _, _)| id).collect();
    ids.sort();
    for id in ids {
        writeln!(out, "param {id}").unwrap();
    }
    for key in params.serialize_fields().into_keys() {
        writeln!(out, "persist {key}").unwrap();
    }
    out
}

/// Compares [`identity`] with the plugin's `ids.txt`. With `UPDATE_IDS=1` it writes the file
/// instead: do that only for an id that is added, never to accept one that changed.
pub fn check<P: ClapPlugin + Vst3Plugin>(manifest_dir: &str, bundler_toml: &str, package: &str) {
    let path = std::path::Path::new(manifest_dir).join("ids.txt");
    let now = identity::<P>(bundler_toml, package);
    if std::env::var_os("UPDATE_IDS").is_some() {
        std::fs::write(&path, &now).unwrap();
        return;
    }
    let saved = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
    let removed: Vec<&str> =
        saved.lines().filter(|line| !now.lines().any(|n| n == *line)).collect();
    assert!(
        removed.is_empty(),
        "ids that saved projects use are gone or changed: {removed:?}. They must stay as they are \
         (Versioning in CLAUDE.md)"
    );
    assert_eq!(
        saved, now,
        "ids were added: run `UPDATE_IDS=1 cargo test -p {package} ids` and commit ids.txt"
    );
}

fn bundle_name(bundler_toml: &str, package: &str) -> String {
    let header = format!("[{package}]");
    let mut lines = bundler_toml.lines().skip_while(|line| line.trim() != header).skip(1);
    lines
        .find_map(|line| {
            line.trim().strip_prefix("name = ").map(|name| name.trim_matches('"').to_string())
        })
        .unwrap_or_else(|| panic!("no bundle name for {package} in bundler.toml"))
}

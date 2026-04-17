use std::path::PathBuf;
use std::process::Command;

fn main() {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let ui_dist = manifest.join("../../ui/dist");
    let ui_dir = manifest.join("../../ui");
    println!("cargo:rerun-if-changed=../../ui/src");
    println!("cargo:rerun-if-changed=../../ui/package.json");

    if !ui_dist.exists() {
        // CI should build UI first. Best-effort fallback.
        let status = Command::new("npm")
            .args(["run", "build"])
            .current_dir(&ui_dir)
            .status();
        match status {
            Ok(s) if s.success() => {}
            _ => {
                // Create empty dir so rust-embed doesn't fail.
                std::fs::create_dir_all(&ui_dist).ok();
                std::fs::write(
                    ui_dist.join("index.html"),
                    "<!doctype html><html><body>UI not built. Run `npm run build` in ui/.</body></html>",
                )
                .ok();
            }
        }
    }
}

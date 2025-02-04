use std::fs;
use std::os::unix::fs::symlink;
use std::path::{Path, PathBuf};

fn main() {
    let script_dir = Path::new("scripts");
    let bin_dir = Path::new("src/bin");

    if !bin_dir.exists() {
        fs::create_dir_all(bin_dir).expect("Failed to create src/bin directory");
    }
    if !script_dir.exists() {
        fs::create_dir_all(script_dir).expect("Failed to create script directory");
    }

    for entry in fs::read_dir(script_dir).expect("Failed to read scripts directory") {
        let entry = entry.expect("Failed to get entry");
        let script_path = entry.path();

        if script_path.extension().and_then(|s| s.to_str()) == Some("rs") {
            let script_name = script_path.file_stem().unwrap().to_str().unwrap();
            let symlink_path = bin_dir.join(format!("{}.rs", script_name));

            // Remove existing symlink to avoid conflicts
            let _ = fs::remove_file(&symlink_path);

            // Compute a relative path
            let relative_script_path = PathBuf::from("../..").join(script_path);

            symlink(&relative_script_path, &symlink_path).expect("Failed to create symlink");
        }
    }
}

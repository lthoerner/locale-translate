use std::path::{Path, PathBuf};

use soft_canonicalize::soft_canonicalize;

use crate::APP_DIR_PATH;

pub fn exit(message: &str) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

pub fn file_exists(path: &Path) -> bool {
    let Ok(path) = soft_canonicalize(path) else {
        exit("Provided path was malformed.");
    };

    path.exists()
}

pub fn create_app_directory_if_not_exists() {
    if PathBuf::from(APP_DIR_PATH).exists() {
        return;
    }

    if std::fs::create_dir(APP_DIR_PATH).is_err() {
        exit(
            "Failed to create or write to ltranslate directory. Ensure that the file permissions are set correctly.",
        );
    }
}

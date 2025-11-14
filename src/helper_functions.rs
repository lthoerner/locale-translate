use std::path::{Path, PathBuf};

use soft_canonicalize::soft_canonicalize;

use crate::exit;

pub fn file_exists(path: &Path) -> bool {
    let Ok(path) = soft_canonicalize(path) else {
        exit!("Provided path was malformed.");
    };

    path.exists()
}

pub fn create_directory_if_not_exists(path: impl Into<PathBuf>) {
    let path = path.into();
    if path.exists() {
        return;
    }

    if std::fs::create_dir_all(&path).is_err() {
        exit!(
            "Failed to create directory '{}'. Ensure that the file permissions are set correctly.",
            path.to_string_lossy()
        );
    }
}

pub fn create_parent_directories_if_not_exists(path: impl Into<PathBuf>) {
    let path = path.into();
    let Some(parent) = path.parent() else {
        return;
    };

    if parent.exists() {
        return;
    }

    if std::fs::create_dir_all(parent).is_err() {
        exit!(
            "Failed to create parent directories for '{}'. Ensure that the file permissions are set correctly.",
            path.to_string_lossy()
        );
    }
}

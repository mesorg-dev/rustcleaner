use std::{
    collections::HashMap,
    fs::File,
    io::{BufReader, Read},
    path::{Path, PathBuf},
};

use walkdir::WalkDir;

pub const LARGE_INSTALLER_THRESHOLD: u64 = 100 * 1024 * 1024;

const INSTALLER_EXTENSIONS: &[&str] = &[
    "aab",
    "apk",
    "appx",
    "appxbundle",
    "deb",
    "dmg",
    "exe",
    "img",
    "iso",
    "msi",
    "msix",
    "pkg",
    "rpm",
];

#[derive(Clone, Debug)]
pub struct ScannedFile {
    pub path: PathBuf,
    pub display_path: String,
    pub name: String,
    pub extension: String,
    pub size: u64,
    pub duplicate: bool,
    pub large_installer: bool,
    pub selected: bool,
}

#[cfg(any(target_os = "android", test))]
pub struct RemoteFile {
    pub uri: String,
    pub display_path: String,
    pub name: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct ScanResult {
    pub files: Vec<ScannedFile>,
    pub errors: Vec<String>,
}

pub fn scan_folder(root: &Path) -> Result<ScanResult, String> {
    if !root.is_dir() {
        return Err(format!(
            "Папка не найдена или недоступна: {}",
            root.display()
        ));
    }

    let mut files = Vec::new();
    let mut errors = Vec::new();

    for entry in WalkDir::new(root).follow_links(false) {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                record_error(&mut errors, error.to_string());
                continue;
            }
        };

        if !entry.file_type().is_file() {
            continue;
        }

        let metadata = match entry.metadata() {
            Ok(metadata) => metadata,
            Err(error) => {
                record_error(&mut errors, format!("{}: {error}", entry.path().display()));
                continue;
            }
        };

        let path = entry.into_path();
        let name = path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        let extension = extension_of(&name);
        let size = metadata.len();
        let large_installer = is_large_installer(&extension, size);
        let display_path = path.display().to_string();

        files.push(ScannedFile {
            path,
            display_path,
            name,
            extension,
            size,
            duplicate: false,
            large_installer,
            selected: false,
        });
    }

    mark_duplicates(&mut files, &mut errors);

    Ok(ScanResult { files, errors })
}

#[cfg(any(target_os = "android", test))]
pub fn scan_remote_files(
    remote_files: Vec<RemoteFile>,
    mut errors: Vec<String>,
    mut hash_file: impl FnMut(&str) -> Result<String, String>,
) -> ScanResult {
    let mut files = Vec::with_capacity(remote_files.len());
    let mut same_size: HashMap<u64, Vec<usize>> = HashMap::new();

    for remote in remote_files {
        let extension = extension_of(&remote.name);
        let index = files.len();
        same_size.entry(remote.size).or_default().push(index);
        files.push(ScannedFile {
            path: PathBuf::from(remote.uri),
            display_path: remote.display_path,
            name: remote.name,
            large_installer: is_large_installer(&extension, remote.size),
            extension,
            size: remote.size,
            duplicate: false,
            selected: false,
        });
    }

    let mut hashes = HashMap::new();
    for (size, indices) in same_size.into_iter().filter(|(_, group)| group.len() > 1) {
        for index in indices {
            match hash_file(files[index].path.to_string_lossy().as_ref()) {
                Ok(hash) => hashes
                    .entry((size, hash))
                    .or_insert_with(Vec::new)
                    .push(index),
                Err(error) => record_error(
                    &mut errors,
                    format!("{}: {error}", files[index].display_path),
                ),
            }
        }
    }

    mark_duplicate_groups(&mut files, hashes);
    ScanResult { files, errors }
}

fn record_error(errors: &mut Vec<String>, error: String) {
    if errors.len() < 5 {
        errors.push(error);
    }
}

fn mark_duplicates(files: &mut [ScannedFile], errors: &mut Vec<String>) {
    let mut same_size: HashMap<u64, Vec<usize>> = HashMap::new();
    for (index, file) in files.iter().enumerate() {
        same_size.entry(file.size).or_default().push(index);
    }

    let mut hashes_by_size: HashMap<(u64, blake3::Hash), Vec<usize>> = HashMap::new();
    for (size, indices) in same_size
        .into_iter()
        .filter(|(_, indices)| indices.len() > 1)
    {
        for index in indices {
            match hash_file(&files[index].path) {
                Ok(hash) => hashes_by_size.entry((size, hash)).or_default().push(index),
                Err(error) => {
                    record_error(errors, format!("{}: {error}", files[index].path.display()))
                }
            }
        }
    }

    mark_duplicate_groups(files, hashes_by_size);
}

fn mark_duplicate_groups<K: Eq + std::hash::Hash>(
    files: &mut [ScannedFile],
    groups: HashMap<K, Vec<usize>>,
) {
    for duplicates in groups.into_values().filter(|group| group.len() > 1) {
        for index in duplicates {
            files[index].duplicate = true;
        }
    }
}

fn extension_of(name: &str) -> String {
    Path::new(name)
        .extension()
        .map(|extension| format!(".{}", extension.to_string_lossy().to_lowercase()))
        .unwrap_or_else(|| "(без расширения)".to_owned())
}

fn is_large_installer(extension: &str, size: u64) -> bool {
    size >= LARGE_INSTALLER_THRESHOLD
        && INSTALLER_EXTENSIONS
            .iter()
            .any(|candidate| extension == format!(".{candidate}"))
}

fn hash_file(path: &Path) -> std::io::Result<blake3::Hash> {
    let mut reader = BufReader::new(File::open(path)?);
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0; 64 * 1024];

    loop {
        let bytes_read = reader.read(&mut buffer)?;
        if bytes_read == 0 {
            break;
        }
        hasher.update(&buffer[..bytes_read]);
    }

    Ok(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{scan_folder, LARGE_INSTALLER_THRESHOLD};

    #[test]
    fn detects_duplicates_by_content_and_ignores_same_size_different_content() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("first.dat"), b"same bytes").unwrap();
        fs::write(directory.path().join("copy.dat"), b"same bytes").unwrap();
        fs::write(directory.path().join("different.dat"), b"other bytes").unwrap();

        let result = scan_folder(directory.path()).unwrap();

        assert_eq!(result.files.iter().filter(|file| file.duplicate).count(), 2);
        assert!(
            !result
                .files
                .iter()
                .find(|file| file.name == "different.dat")
                .unwrap()
                .duplicate
        );
    }

    #[test]
    fn marks_only_large_known_installer_types() {
        let directory = tempfile::tempdir().unwrap();
        let installer = directory.path().join("setup.exe");
        let archive = directory.path().join("archive.zip");
        let file = fs::File::create(&installer).unwrap();
        file.set_len(LARGE_INSTALLER_THRESHOLD).unwrap();
        let file = fs::File::create(&archive).unwrap();
        file.set_len(LARGE_INSTALLER_THRESHOLD).unwrap();

        let result = scan_folder(directory.path()).unwrap();

        assert!(
            result
                .files
                .iter()
                .find(|file| file.name == "setup.exe")
                .unwrap()
                .large_installer
        );
        assert!(
            !result
                .files
                .iter()
                .find(|file| file.name == "archive.zip")
                .unwrap()
                .large_installer
        );
    }

    #[test]
    fn groups_remote_files_by_content_hash() {
        let files = vec![
            super::RemoteFile {
                uri: "content://provider/one".to_owned(),
                display_path: "Downloads/one.apk".to_owned(),
                name: "one.apk".to_owned(),
                size: 12,
            },
            super::RemoteFile {
                uri: "content://provider/two".to_owned(),
                display_path: "Downloads/two.apk".to_owned(),
                name: "two.apk".to_owned(),
                size: 12,
            },
            super::RemoteFile {
                uri: "content://provider/three".to_owned(),
                display_path: "Downloads/three.apk".to_owned(),
                name: "three.apk".to_owned(),
                size: 12,
            },
        ];

        let result = super::scan_remote_files(files, Vec::new(), |uri| {
            Ok(if uri.ends_with("three") {
                "different".to_owned()
            } else {
                "same".to_owned()
            })
        });

        assert_eq!(result.files.iter().filter(|file| file.duplicate).count(), 2);
        assert_eq!(result.files[0].display_path, "Downloads/one.apk");
    }
}

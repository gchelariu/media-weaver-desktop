use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateNameReport {
    scanned_files: usize,
    duplicate_names: usize,
    duplicate_files: usize,
    directories: Vec<DirectoryDuplicateGroup>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryDuplicateGroup {
    directory: String,
    files: Vec<DuplicateFile>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DuplicateFile {
    name: String,
    duplicate_directories: Vec<String>,
}

pub fn scan(root: &str) -> Result<DuplicateNameReport, String> {
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Err(format!("Not a directory: {}", root.display()));
    }

    let mut files = Vec::new();
    let mut errors = Vec::new();
    collect_files(&root, &mut files, &mut errors);
    let scanned_files = files.len();
    let (duplicate_names, duplicate_files, directories) = group_duplicate_names(files);

    Ok(DuplicateNameReport {
        scanned_files,
        duplicate_names,
        duplicate_files,
        directories,
        errors,
    })
}

fn collect_files(directory: &Path, files: &mut Vec<(OsString, PathBuf)>, errors: &mut Vec<String>) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            errors.push(format!("Could not read {}: {error}", directory.display()));
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                errors.push(format!(
                    "Could not read an entry in {}: {error}",
                    directory.display()
                ));
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                errors.push(format!("Could not inspect {}: {error}", path.display()));
                continue;
            }
        };

        if file_type.is_dir() {
            collect_files(&path, files, errors);
        } else if file_type.is_file() {
            if let Some(name) = path.file_name() {
                files.push((name.to_os_string(), path));
            }
        }
    }
}

fn group_duplicate_names(
    files: Vec<(OsString, PathBuf)>,
) -> (usize, usize, Vec<DirectoryDuplicateGroup>) {
    let mut by_name: BTreeMap<OsString, Vec<PathBuf>> = BTreeMap::new();
    for (name, path) in files {
        by_name.entry(name).or_default().push(path);
    }

    let duplicate_groups: Vec<_> = by_name
        .into_iter()
        .filter_map(|(name, mut paths)| {
            if paths.len() < 2 {
                return None;
            }
            paths.sort();
            Some((name, paths))
        })
        .collect();
    let duplicate_names = duplicate_groups.len();
    let duplicate_files = duplicate_groups.iter().map(|(_, paths)| paths.len()).sum();
    let mut by_directory: BTreeMap<PathBuf, Vec<DuplicateFile>> = BTreeMap::new();

    for (name, paths) in duplicate_groups {
        for path in &paths {
            let Some(directory) = path.parent() else {
                continue;
            };
            let duplicate_directories: BTreeSet<_> = paths
                .iter()
                .filter_map(|other_path| other_path.parent())
                .filter(|other_directory| *other_directory != directory)
                .map(Path::to_path_buf)
                .collect();

            by_directory
                .entry(directory.to_path_buf())
                .or_default()
                .push(DuplicateFile {
                    name: name.to_string_lossy().into_owned(),
                    duplicate_directories: duplicate_directories
                        .into_iter()
                        .map(|path| path.to_string_lossy().into_owned())
                        .collect(),
                });
        }
    }

    let directories = by_directory
        .into_iter()
        .map(|(directory, files)| DirectoryDuplicateGroup {
            directory: directory.to_string_lossy().into_owned(),
            files,
        })
        .collect();

    (duplicate_names, duplicate_files, directories)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_matches_by_directory_and_ignores_unique_names() {
        let (duplicate_names, duplicate_files, directories) = group_duplicate_names(vec![
            ("photo.jpg".into(), PathBuf::from("/photos/one/photo.jpg")),
            ("photo.jpg".into(), PathBuf::from("/photos/two/photo.jpg")),
            ("trip.jpg".into(), PathBuf::from("/photos/one/trip.jpg")),
            ("trip.jpg".into(), PathBuf::from("/photos/three/trip.jpg")),
            ("Photo.jpg".into(), PathBuf::from("/photos/Photo.jpg")),
            ("notes.txt".into(), PathBuf::from("/photos/notes.txt")),
        ]);

        assert_eq!(duplicate_names, 2);
        assert_eq!(duplicate_files, 4);
        assert_eq!(directories.len(), 3);
        assert_eq!(directories[0].directory, "/photos/one");
        assert_eq!(directories[0].files.len(), 2);
        assert_eq!(directories[0].files[0].name, "photo.jpg");
        assert_eq!(
            directories[0].files[0].duplicate_directories,
            ["/photos/two"]
        );
        assert_eq!(directories[1].directory, "/photos/three");
        assert_eq!(
            directories[1].files[0].duplicate_directories,
            ["/photos/one"]
        );
        assert_eq!(directories[2].directory, "/photos/two");
        assert_eq!(
            directories[2].files[0].duplicate_directories,
            ["/photos/one"]
        );
    }
}

use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::{Component, Path, PathBuf};

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
pub struct DeleteDuplicateReport {
    deleted_files: usize,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryDuplicateGroup {
    directory: String,
    duplicate_directories: Vec<DirectoryMatchGroup>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DirectoryMatchGroup {
    directory: String,
    file_names: Vec<String>,
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

pub fn delete_duplicates(
    root: &str,
    directory: &str,
    file_names: Vec<String>,
) -> Result<DeleteDuplicateReport, String> {
    if file_names.is_empty() {
        return Err("No duplicate files were selected for deletion".into());
    }

    let canonical_root = PathBuf::from(root)
        .canonicalize()
        .map_err(|error| format!("Could not access scan folder: {error}"))?;
    if !canonical_root.is_dir() {
        return Err(format!("Not a directory: {}", canonical_root.display()));
    }

    let canonical_directory = PathBuf::from(directory)
        .canonicalize()
        .map_err(|error| format!("Could not access target directory: {error}"))?;
    if !canonical_directory.starts_with(&canonical_root) || !canonical_directory.is_dir() {
        return Err("Target directory is not inside the selected scan folder".into());
    }

    // Rescan immediately before deleting so stale UI results cannot authorize
    // removal of files that are no longer duplicates.
    let fresh_report = scan(&canonical_root.to_string_lossy())?;
    let target_directory = canonical_directory.to_string_lossy().into_owned();
    let duplicate_names: BTreeSet<String> = fresh_report
        .directories
        .iter()
        .flat_map(|source| source.duplicate_directories.iter())
        .filter(|duplicate| duplicate.directory == target_directory)
        .flat_map(|duplicate| duplicate.file_names.iter().cloned())
        .collect();

    let requested_names: BTreeSet<String> = file_names.into_iter().collect();
    let mut paths_to_delete = Vec::with_capacity(requested_names.len());
    for name in requested_names {
        let name_path = Path::new(&name);
        let mut components = name_path.components();
        if !matches!(components.next(), Some(Component::Normal(component)) if component == OsStr::new(&name))
            || components.next().is_some()
        {
            return Err(format!("Invalid file name: {name}"));
        }
        if !duplicate_names.contains(&name) {
            return Err(format!(
                "'{name}' is no longer confirmed as a duplicate in {}",
                canonical_directory.display()
            ));
        }

        let path = canonical_directory.join(&name);
        let metadata = fs::symlink_metadata(&path)
            .map_err(|error| format!("Could not inspect {}: {error}", path.display()))?;
        if !metadata.file_type().is_file() {
            return Err(format!("Not a regular file: {}", path.display()));
        }
        let canonical_path = path
            .canonicalize()
            .map_err(|error| format!("Could not access {}: {error}", path.display()))?;
        if canonical_path.parent() != Some(canonical_directory.as_path()) {
            return Err(format!(
                "File is not directly inside the target directory: {}",
                path.display()
            ));
        }
        paths_to_delete.push(path);
    }

    let mut report = DeleteDuplicateReport {
        deleted_files: 0,
        errors: Vec::new(),
    };
    for path in paths_to_delete {
        match fs::remove_file(&path) {
            Ok(()) => report.deleted_files += 1,
            Err(error) => report
                .errors
                .push(format!("Could not delete {}: {error}", path.display())),
        }
    }
    Ok(report)
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
    let mut by_directory: BTreeMap<PathBuf, BTreeMap<PathBuf, BTreeSet<OsString>>> =
        BTreeMap::new();

    for (name, paths) in duplicate_groups {
        for path in &paths {
            let Some(directory) = path.parent() else {
                continue;
            };
            for duplicate_directory in paths
                .iter()
                .filter_map(|other_path| other_path.parent())
                .filter(|other_directory| *other_directory != directory)
            {
                by_directory
                    .entry(directory.to_path_buf())
                    .or_default()
                    .entry(duplicate_directory.to_path_buf())
                    .or_default()
                    .insert(name.clone());
            }
        }
    }

    let directories = by_directory
        .into_iter()
        .map(
            |(directory, duplicate_directories)| DirectoryDuplicateGroup {
                directory: directory.to_string_lossy().into_owned(),
                duplicate_directories: duplicate_directories
                    .into_iter()
                    .map(|(directory, file_names)| DirectoryMatchGroup {
                        directory: directory.to_string_lossy().into_owned(),
                        file_names: file_names
                            .into_iter()
                            .map(|name| name.to_string_lossy().into_owned())
                            .collect(),
                    })
                    .collect(),
            },
        )
        .collect();

    (duplicate_names, duplicate_files, directories)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn unique_temp_dir() -> PathBuf {
        static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
        std::env::temp_dir().join(format!(
            "media-weaver-duplicate-test-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ))
    }

    #[test]
    fn groups_matches_by_directory_and_ignores_unique_names() {
        let (duplicate_names, duplicate_files, directories) = group_duplicate_names(vec![
            ("photo.jpg".into(), PathBuf::from("/photos/one/photo.jpg")),
            ("photo.jpg".into(), PathBuf::from("/photos/two/photo.jpg")),
            ("trip.jpg".into(), PathBuf::from("/photos/one/trip.jpg")),
            ("trip.jpg".into(), PathBuf::from("/photos/two/trip.jpg")),
            ("trip.jpg".into(), PathBuf::from("/photos/three/trip.jpg")),
            ("Photo.jpg".into(), PathBuf::from("/photos/Photo.jpg")),
            ("notes.txt".into(), PathBuf::from("/photos/notes.txt")),
        ]);

        assert_eq!(duplicate_names, 2);
        assert_eq!(duplicate_files, 5);
        assert_eq!(directories.len(), 3);
        assert_eq!(directories[0].directory, "/photos/one");
        assert_eq!(directories[0].duplicate_directories.len(), 2);
        assert_eq!(
            directories[0].duplicate_directories[0].directory,
            "/photos/three"
        );
        assert_eq!(
            directories[0].duplicate_directories[0].file_names,
            ["trip.jpg"]
        );
        assert_eq!(
            directories[0].duplicate_directories[1].directory,
            "/photos/two"
        );
        assert_eq!(
            directories[0].duplicate_directories[1].file_names,
            ["photo.jpg", "trip.jpg"]
        );
        assert_eq!(directories[1].directory, "/photos/three");
        assert_eq!(
            directories[1].duplicate_directories[0].file_names,
            ["trip.jpg"]
        );
        assert_eq!(directories[2].directory, "/photos/two");
        assert_eq!(directories[2].duplicate_directories.len(), 2);
        assert_eq!(
            directories[2].duplicate_directories[0].file_names,
            ["photo.jpg", "trip.jpg"]
        );
        assert_eq!(
            directories[2].duplicate_directories[1].file_names,
            ["trip.jpg"]
        );
    }

    #[test]
    fn deletes_only_files_still_confirmed_as_duplicates_in_target_directory() {
        let root = unique_temp_dir();
        let original = root.join("originals");
        let duplicate = root.join("copies");
        fs::create_dir_all(&original).unwrap();
        fs::create_dir_all(&duplicate).unwrap();
        fs::write(original.join("photo.jpg"), b"original").unwrap();
        fs::write(duplicate.join("photo.jpg"), b"duplicate").unwrap();
        fs::write(duplicate.join("unique.jpg"), b"unique").unwrap();

        let report = delete_duplicates(
            &root.to_string_lossy(),
            &duplicate.to_string_lossy(),
            vec!["photo.jpg".into()],
        )
        .unwrap();
        assert_eq!(report.deleted_files, 1);
        assert!(original.join("photo.jpg").exists());
        assert!(!duplicate.join("photo.jpg").exists());

        let result = delete_duplicates(
            &root.to_string_lossy(),
            &duplicate.to_string_lossy(),
            vec!["unique.jpg".into()],
        );
        assert!(result.is_err());
        assert!(duplicate.join("unique.jpg").exists());

        fs::remove_dir_all(root).unwrap();
    }
}

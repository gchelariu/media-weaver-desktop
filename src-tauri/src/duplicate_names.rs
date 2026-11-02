use serde::Serialize;
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateNameReport {
    scanned_files: usize,
    duplicate_groups: Vec<DuplicateNameGroup>,
    errors: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DuplicateNameGroup {
    name: String,
    paths: Vec<String>,
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
    let duplicate_groups = group_duplicate_names(files);

    Ok(DuplicateNameReport {
        scanned_files,
        duplicate_groups,
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

fn group_duplicate_names(files: Vec<(OsString, PathBuf)>) -> Vec<DuplicateNameGroup> {
    let mut by_name: BTreeMap<OsString, Vec<PathBuf>> = BTreeMap::new();
    for (name, path) in files {
        by_name.entry(name).or_default().push(path);
    }

    by_name
        .into_iter()
        .filter_map(|(name, mut paths)| {
            if paths.len() < 2 {
                return None;
            }
            paths.sort();
            Some(DuplicateNameGroup {
                name: name.to_string_lossy().into_owned(),
                paths: paths
                    .into_iter()
                    .map(|path| path.to_string_lossy().into_owned())
                    .collect(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_exact_file_names_and_ignores_unique_names() {
        let groups = group_duplicate_names(vec![
            ("photo.jpg".into(), PathBuf::from("/photos/one/photo.jpg")),
            ("photo.jpg".into(), PathBuf::from("/photos/two/photo.jpg")),
            ("Photo.jpg".into(), PathBuf::from("/photos/Photo.jpg")),
            ("notes.txt".into(), PathBuf::from("/photos/notes.txt")),
        ]);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].name, "photo.jpg");
        assert_eq!(
            groups[0].paths,
            ["/photos/one/photo.jpg", "/photos/two/photo.jpg"]
        );
    }
}

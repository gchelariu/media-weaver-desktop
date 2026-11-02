use serde::Serialize;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tauri::{Emitter, Window};
use tauri_plugin_dialog::DialogExt;

mod duplicate_names;

const IMAGE_DATE_TAGS: &[&str] = &["DateTimeOriginal", "CreateDate", "ModifyDate"];
const VIDEO_DATE_TAGS: &[&str] = &[
    "CreateDate",
    "ModifyDate",
    "TrackCreateDate",
    "TrackModifyDate",
    "MediaCreateDate",
    "MediaModifyDate",
    "DateTimeOriginal",
    "XMP:CreateDate",
    "XMP:DateTimeOriginal",
];
const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "tif", "tiff", "png", "webp", "heic", "heif"];
const VIDEO_EXTENSIONS: &[&str] = &["mov", "mp4", "m4v", "mkv"];
const DATE_FORMAT: &str = "%Y:%m:%d %H:%M:%S";
const DEFAULT_EXIF_DATE: &str = "0001:01:02 12:31:56";

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct FileResult {
    path: String,
    outcome: String,
    dry_run: bool,
    scanned: bool,
    target_date: Option<String>,
    message: String,
}

#[derive(Default, Serialize)]
#[serde(rename_all = "camelCase")]
struct Report {
    scanned: usize,
    changed: usize,
    skipped: usize,
    out_of_structure: usize,
    errors: usize,
    results: Vec<FileResult>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MetadataFields {
    images: Vec<String>,
    videos: Vec<String>,
}

#[derive(Debug)]
struct ExistingDate {
    day: String,
    time: String,
    raw: String,
}

#[tauri::command]
async fn choose_directory(app: tauri::AppHandle) -> Option<String> {
    tauri::async_runtime::spawn_blocking(move || app.dialog().file().blocking_pick_folder())
        .await
        .ok()
        .flatten()
        .map(|path| path.to_string())
}

#[tauri::command]
fn get_metadata_fields() -> MetadataFields {
    fn fields_for(tags: &[&str]) -> Vec<String> {
        tags.iter()
            .copied()
            .chain(std::iter::once("FileModifyDate"))
            .map(str::to_owned)
            .collect()
    }

    MetadataFields {
        images: fields_for(IMAGE_DATE_TAGS),
        videos: fields_for(VIDEO_DATE_TAGS),
    }
}

#[tauri::command]
async fn process_media(window: Window, root: String, dry_run: bool) -> Result<Report, String> {
    tauri::async_runtime::spawn_blocking(move || process_media_sync(window, root, dry_run))
        .await
        .map_err(|error| format!("Media processing task failed: {error}"))?
}

#[tauri::command]
async fn scan_duplicate_names(
    root: String,
) -> Result<duplicate_names::DuplicateNameReport, String> {
    tauri::async_runtime::spawn_blocking(move || duplicate_names::scan(&root))
        .await
        .map_err(|error| format!("Duplicate-name scan task failed: {error}"))?
}

#[tauri::command]
async fn delete_duplicate_files(
    root: String,
    directory: String,
    file_names: Vec<String>,
) -> Result<duplicate_names::DeleteDuplicateReport, String> {
    tauri::async_runtime::spawn_blocking(move || {
        duplicate_names::delete_duplicates(&root, &directory, file_names)
    })
    .await
    .map_err(|error| format!("Duplicate-file deletion task failed: {error}"))?
}

fn process_media_sync(window: Window, root: String, dry_run: bool) -> Result<Report, String> {
    let root = PathBuf::from(root);
    if !root.is_dir() {
        return Err(format!("Not a directory: {}", root.display()));
    }

    Command::new("exiftool")
        .arg("-ver")
        .output()
        .map_err(|error| {
            format!(
                "ExifTool is not installed or could not be started: {error}. Install ExifTool and make sure it is on PATH."
            )
        })?
        .checked("check ExifTool")?;

    let mut report = Report::default();
    walk_and_process(&root, &window, dry_run, &mut report);
    Ok(report)
}

trait CheckedOutput {
    fn checked(self, action: &str) -> Result<(), String>;
}

impl CheckedOutput for Output {
    fn checked(self, action: &str) -> Result<(), String> {
        if self.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&self.stderr).trim().to_owned();
            Err(if stderr.is_empty() {
                format!("ExifTool failed to {action} (exit status {})", self.status)
            } else {
                format!("ExifTool failed to {action}: {stderr}")
            })
        }
    }
}

fn walk_and_process(directory: &Path, window: &Window, dry_run: bool, report: &mut Report) {
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) => {
            report.errors += 1;
            emit_result(
                window,
                report,
                FileResult {
                    path: directory.display().to_string(),
                    outcome: "error".into(),
                    dry_run,
                    scanned: false,
                    target_date: None,
                    message: format!("Could not read directory: {error}"),
                },
            );
            return;
        }
    };

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(error) => {
                report.errors += 1;
                emit_result(
                    window,
                    report,
                    FileResult {
                        path: directory.display().to_string(),
                        outcome: "error".into(),
                        dry_run,
                        scanned: false,
                        target_date: None,
                        message: format!("Could not read directory entry: {error}"),
                    },
                );
                continue;
            }
        };
        let path = entry.path();
        let file_type = match entry.file_type() {
            Ok(file_type) => file_type,
            Err(error) => {
                report.errors += 1;
                emit_result(
                    window,
                    report,
                    FileResult {
                        path: path.display().to_string(),
                        outcome: "error".into(),
                        dry_run,
                        scanned: false,
                        target_date: None,
                        message: format!("Could not inspect file: {error}"),
                    },
                );
                continue;
            }
        };
        if file_type.is_dir() {
            walk_and_process(&path, window, dry_run, report);
        } else if file_type.is_file() {
            if let Some(tags) = media_tags(&path) {
                report.scanned += 1;
                let result = if find_year_month(&path).is_some() {
                    process_file(&path, tags, dry_run)
                } else {
                    FileResult {
                        path: path.display().to_string(),
                        outcome: "unmatched".into(),
                        dry_run,
                        scanned: true,
                        target_date: None,
                        message: folder_layout_issue(&path),
                    }
                };
                match result.outcome.as_str() {
                    "changed" => report.changed += 1,
                    "skipped" => report.skipped += 1,
                    "unmatched" => report.out_of_structure += 1,
                    _ => report.errors += 1,
                }
                emit_result(window, report, result);
            }
        }
    }
}

fn emit_result(window: &Window, report: &mut Report, result: FileResult) {
    let _ = window.emit("media-progress", &result);
    report.results.push(result);
}

fn media_tags(path: &Path) -> Option<&'static [&'static str]> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    if IMAGE_EXTENSIONS.contains(&extension.as_str()) {
        Some(IMAGE_DATE_TAGS)
    } else if VIDEO_EXTENSIONS.contains(&extension.as_str()) {
        Some(VIDEO_DATE_TAGS)
    } else {
        None
    }
}

fn process_file(path: &Path, tags: &[&str], dry_run: bool) -> FileResult {
    let path_string = path.display().to_string();
    match process_file_inner(path, tags, dry_run) {
        Ok((outcome, target_date, message)) => FileResult {
            path: path_string,
            outcome,
            dry_run,
            scanned: true,
            target_date: Some(target_date),
            message,
        },
        Err(error) => FileResult {
            path: path_string,
            outcome: "error".into(),
            dry_run,
            scanned: true,
            target_date: None,
            message: error,
        },
    }
}

fn process_file_inner(
    path: &Path,
    tags: &[&str],
    dry_run: bool,
) -> Result<(String, String, String), String> {
    let metadata = read_metadata(path, tags)?;
    let existing = read_existing_date(&metadata);
    let (year, month_folder) = find_year_month(path)
        .ok_or_else(|| "File is not inside a <YEAR>/<MM...> folder pair".to_owned())?;
    let month = &month_folder[..2];
    let (day, time) = if let Some(existing) = &existing {
        (existing.day.as_str(), existing.time.as_str())
    } else {
        let mut parts = DEFAULT_EXIF_DATE.split_whitespace();
        let date = parts.next().unwrap_or("0001:01:02");
        let time = parts.next().unwrap_or("12:31:56");
        (date.rsplit(':').next().unwrap_or("02"), time)
    };
    let target_date = format!("{year}:{month}:{day} {time}");
    let tags_match = tags
        .iter()
        .all(|tag| read_tag_value(&metadata, tag).as_deref() == Some(target_date.as_str()));
    let file_date_matches =
        read_tag_value(&metadata, "FileModifyDate").as_deref() == Some(target_date.as_str());

    if tags_match && file_date_matches {
        return Ok((
            "skipped".into(),
            target_date.clone(),
            format!("Already matches {target_date} (skipped)"),
        ));
    }

    if dry_run {
        let message = match existing {
            Some(existing) => format!("{} → {target_date}", existing.raw),
            None => format!("No usable date → {target_date} (default date/time)"),
        };
        return Ok(("changed".into(), target_date, message));
    }

    write_metadata_date(path, &target_date, tags)?;
    let written = read_metadata(path, tags)?;
    let tags_written = tags
        .iter()
        .all(|tag| read_tag_value(&written, tag).as_deref() == Some(target_date.as_str()));
    let file_date_written =
        read_tag_value(&written, "FileModifyDate").as_deref() == Some(target_date.as_str());
    if !tags_written || !file_date_written {
        return Err(
            "Post-write verification failed: one or more dates do not match the target".into(),
        );
    }

    let message = match existing {
        Some(existing) => format!("{} → {target_date}", existing.raw),
        None => format!("No usable date → {target_date} (default date/time)"),
    };
    Ok(("changed".into(), target_date, message))
}

fn find_year_month(path: &Path) -> Option<(&str, &str)> {
    let month = path.parent()?.file_name()?.to_str()?;
    let year = path.parent()?.parent()?.file_name()?.to_str()?;
    (is_year(year) && has_valid_month_prefix(month)).then_some((year, month))
}

fn folder_layout_issue(path: &Path) -> String {
    let parent = path.parent();
    let parent_name = parent
        .and_then(Path::file_name)
        .and_then(|name| name.to_str())
        .unwrap_or("unknown folder");

    if is_year(parent_name) {
        return format!(
            "Missing month folder under year folder '{parent_name}'. Expected a path like {parent_name}/05-Holiday/<file>."
        );
    }

    let year_name = parent
        .and_then(Path::parent)
        .and_then(Path::file_name)
        .and_then(|name| name.to_str());
    if let Some(year) = year_name.filter(|year| is_year(year)) {
        return format!(
            "'{parent_name}' is not a valid month folder under year '{year}'. Use a folder beginning with 01–12, such as 05-Holiday."
        );
    }

    let ancestor_year = path
        .ancestors()
        .skip(1)
        .filter_map(Path::file_name)
        .filter_map(|name| name.to_str())
        .find(|name| is_year(name));
    if let Some(year) = ancestor_year {
        return format!(
            "File is not directly inside a month folder under year '{year}'. Expected <YEAR>/<MM...>/<file>."
        );
    }

    "No valid four-digit year folder was found. Expected <YEAR>/<MM...>/<file>.".into()
}

fn is_year(value: &str) -> bool {
    value.len() == 4 && value.bytes().all(|byte| byte.is_ascii_digit())
}

fn has_valid_month_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2
        && bytes[0].is_ascii_digit()
        && bytes[1].is_ascii_digit()
        && (1..=12).contains(&((bytes[0] - b'0') * 10 + (bytes[1] - b'0')))
}

fn read_metadata(path: &Path, tags: &[&str]) -> Result<Value, String> {
    let mut tags_to_read = vec!["DateTimeOriginal", "CreateDate", "FileModifyDate"];
    for tag in tags {
        if !tags_to_read.contains(tag) {
            tags_to_read.push(tag);
        }
    }
    let mut command = Command::new("exiftool");
    command.args(["-json", "-G1", "-d", DATE_FORMAT]);
    for tag in tags_to_read {
        command.arg(format!("-{tag}"));
    }
    let output = command
        .arg(path)
        .output()
        .map_err(|error| format!("Could not start ExifTool while reading metadata: {error}"))?;
    if !output.status.success() {
        return Err(exiftool_error("read metadata", &output));
    }
    let values: Value = serde_json::from_slice(&output.stdout)
        .map_err(|error| format!("Could not parse ExifTool metadata: {error}"))?;
    Ok(values
        .as_array()
        .and_then(|entries| entries.first())
        .cloned()
        .unwrap_or_else(|| Value::Object(Default::default())))
}

fn read_existing_date(metadata: &Value) -> Option<ExistingDate> {
    for tag in ["DateTimeOriginal", "CreateDate"] {
        let Some(raw) = read_tag_value(metadata, tag) else {
            continue;
        };
        let Some((date, time)) = raw.split_once(' ') else {
            continue;
        };
        let parts: Vec<&str> = date.split(':').collect();
        if parts.len() == 3
            && parts[0].len() == 4
            && parts[1].len() == 2
            && parts[2].len() == 2
            && parts
                .iter()
                .all(|part| part.bytes().all(|byte| byte.is_ascii_digit()))
            && time.len() == 8
            && time.bytes().enumerate().all(|(index, byte)| {
                if [2, 5].contains(&index) {
                    byte == b':'
                } else {
                    byte.is_ascii_digit()
                }
            })
        {
            return Some(ExistingDate {
                day: parts[2].to_owned(),
                time: time.to_owned(),
                raw,
            });
        }
    }
    None
}

fn read_tag_value(metadata: &Value, tag: &str) -> Option<String> {
    let object = metadata.as_object()?;
    let is_xmp = tag.starts_with("XMP:");
    let tag_name = tag.strip_prefix("XMP:").unwrap_or(tag);
    let mut values = object
        .iter()
        .filter(|(key, _)| {
            key.ends_with(&format!(":{tag_name}")) && key.starts_with("XMP-") == is_xmp
        })
        .filter_map(|(_, value)| value.as_str());
    let first = values.next()?;
    if values.any(|value| value != first) {
        return None;
    }
    Some(first.to_owned())
}

fn write_metadata_date(path: &Path, target_date: &str, tags: &[&str]) -> Result<(), String> {
    let mut command = Command::new("exiftool");
    command.args(["-overwrite_original", "-m", "-api", "IgnoreMinorErrors=1"]);
    for tag in tags {
        command.arg(format!("-{tag}={target_date}"));
    }
    let output = command
        .arg(path)
        .output()
        .map_err(|error| format!("Could not start ExifTool while writing metadata: {error}"))?;
    if !output.status.success() {
        // Unlike the CLI's older fallback, never erase all EXIF data when the
        // file has a malformed structure. Report the failure without retrying.
        return Err(exiftool_error("write metadata", &output));
    }

    let output = Command::new("exiftool")
        .args([
            "-overwrite_original",
            &format!("-FileModifyDate={target_date}"),
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Could not start ExifTool to set file date: {error}"))?;
    if !output.status.success() {
        return Err(exiftool_error("set FileModifyDate", &output));
    }
    Ok(())
}

fn exiftool_error(action: &str, output: &Output) -> String {
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
    if stderr.is_empty() {
        format!(
            "ExifTool failed to {action} (exit status {})",
            output.status
        )
    } else {
        format!("ExifTool failed to {action}: {stderr}")
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            choose_directory,
            get_metadata_fields,
            process_media,
            scan_duplicate_names,
            delete_duplicate_files
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn accepts_year_and_month_prefix_with_suffix() {
        assert!(is_year("2024"));
        assert!(!is_year("24"));
        assert!(has_valid_month_prefix("05-Holiday"));
        assert!(!has_valid_month_prefix("13-Invalid"));
        assert!(!has_valid_month_prefix("00"));
    }

    #[test]
    fn exposes_the_configured_metadata_fields() {
        let fields = get_metadata_fields();
        assert_eq!(
            fields.images,
            [
                "DateTimeOriginal",
                "CreateDate",
                "ModifyDate",
                "FileModifyDate"
            ]
        );
        assert_eq!(
            fields.videos,
            [
                "CreateDate",
                "ModifyDate",
                "TrackCreateDate",
                "TrackModifyDate",
                "MediaCreateDate",
                "MediaModifyDate",
                "DateTimeOriginal",
                "XMP:CreateDate",
                "XMP:DateTimeOriginal",
                "FileModifyDate"
            ]
        );
    }

    #[test]
    fn recognizes_only_files_directly_inside_year_month_pair() {
        assert_eq!(
            find_year_month(Path::new("/photos/2024/05-trip/photo.jpg")),
            Some(("2024", "05-trip"))
        );
        assert_eq!(
            find_year_month(Path::new("/photos/2024/05-trip/edited/photo.jpg")),
            None
        );
    }

    #[test]
    fn explains_missing_month_folder() {
        let issue = folder_layout_issue(Path::new("/photos/2010/IMG_4435.jpg"));
        assert!(issue.contains("Missing month folder"));
        assert!(issue.contains("2010/05-Holiday"));
    }

    #[test]
    fn resolves_group_qualified_and_xmp_tags() {
        let metadata = json!({
            "EXIF:DateTimeOriginal": "2014:05:29 11:37:31",
            "XMP-xmp:CreateDate": "2014:05:29 11:37:31",
            "XMP-exif:DateTimeOriginal": "2014:05:29 11:37:31"
        });
        assert_eq!(
            read_tag_value(&metadata, "DateTimeOriginal").as_deref(),
            Some("2014:05:29 11:37:31")
        );
        assert_eq!(
            read_tag_value(&metadata, "XMP:CreateDate").as_deref(),
            Some("2014:05:29 11:37:31")
        );
        assert_eq!(
            read_tag_value(&metadata, "XMP:DateTimeOriginal").as_deref(),
            Some("2014:05:29 11:37:31")
        );
    }

    #[test]
    fn rejects_conflicting_duplicate_tag_values() {
        let metadata = json!({
            "Track1:TrackCreateDate": "2014:05:29 11:37:31",
            "Track2:TrackCreateDate": "2014:05:29 11:37:32"
        });
        assert_eq!(read_tag_value(&metadata, "TrackCreateDate"), None);
    }

    #[test]
    fn selects_date_time_from_existing_metadata() {
        let metadata = json!({
            "EXIF:DateTimeOriginal": "2014:05:29 11:37:31",
            "EXIF:CreateDate": "2014:05:28 10:00:00"
        });
        let existing = read_existing_date(&metadata).unwrap();
        assert_eq!(existing.day, "29");
        assert_eq!(existing.time, "11:37:31");
        assert_eq!(existing.raw, "2014:05:29 11:37:31");
    }
}

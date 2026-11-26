use crate::utils;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use tokio::process::Command;

const VIDEO_MUXERS: &[&str] = &[
    "3g2",
    "3gp",
    "amv",
    "asf",
    "avi",
    "dv",
    "flv",
    "gif",
    "gxf",
    "h261",
    "h263",
    "h264",
    "hevc",
    "ivf",
    "matroska",
    "m4v",
    "mjpeg",
    "mov",
    "mp4",
    "mpeg",
    "mpeg1video",
    "mpeg2video",
    "mpegts",
    "mxf",
    "mxf_d10",
    "mxf_op1a",
    "nut",
    "ogg",
    "ogv",
    "psp",
    "rawvideo",
    "rm",
    "swf",
    "vob",
    "webm",
    "yuv4mpegpipe",
];

#[derive(Serialize)]
pub struct MediaOutputFormat {
    pub format: String,
    pub extension: String,
}

#[derive(Serialize)]
pub struct MediaConversionResult {
    pub input: String,
    pub output: Option<String>,
    pub status: String,
    pub error: Option<String>,
}

#[derive(Clone, Serialize)]
struct MediaConversionProgress {
    run_id: String,
    current: usize,
    total: usize,
    file: String,
    status: String,
    output: Option<String>,
    error: Option<String>,
}

fn output_extension(format: &str) -> &str {
    match format {
        "matroska" => "mkv",
        "mpegts" => "ts",
        "mpeg" => "mpg",
        "ipod" => "m4a",
        "adts" => "aac",
        "oga" => "ogg",
        "mpeg1video" | "mpeg2video" => "mpg",
        "yuv4mpegpipe" => "y4m",
        "rawvideo" => "raw",
        other => other,
    }
}

fn input_extensions(format: &str) -> Vec<String> {
    let aliases: &[&str] = match format {
        "matroska" => &["mkv", "mka", "mks"],
        "mov" => &["mov", "mp4", "m4a", "m4v", "3gp", "3g2", "mj2"],
        "mpegts" => &["ts", "mts", "m2ts"],
        "mpeg" => &["mpg", "mpeg", "mpe"],
        "ipod" => &["m4a", "m4v"],
        "adts" => &["aac"],
        "oga" => &["oga", "ogg"],
        other => return vec![other.to_owned()],
    };
    aliases.iter().map(|alias| (*alias).to_owned()).collect()
}

#[tauri::command]
pub async fn tool_get_media_output_formats(
    app: AppHandle,
) -> Result<Vec<MediaOutputFormat>, String> {
    let ffmpeg = utils::get_ffmpeg_path(&app)?;
    let mut command = Command::new(ffmpeg);
    command.args(["-hide_banner", "-muxers"]).kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(crate::commands::CREATE_NO_WINDOW);
    let output = command
        .output()
        .await
        .map_err(|error| format!("err_run_ffmpeg:{error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }

    let listing = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut formats = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = fields.next()?;
            if !flags.contains('E') {
                return None;
            }
            fields.next().map(str::to_owned)
        })
        .flat_map(|names| names.split(',').map(str::to_owned).collect::<Vec<_>>())
        .filter(|format| VIDEO_MUXERS.contains(&format.as_str()))
        .filter(|format| !format.is_empty() && format.chars().all(|c| c.is_ascii_alphanumeric()))
        .map(|format| MediaOutputFormat {
            extension: output_extension(&format).to_owned(),
            format,
        })
        .collect::<Vec<_>>();
    formats.sort_by(|a, b| a.extension.cmp(&b.extension).then(a.format.cmp(&b.format)));
    formats.dedup_by(|a, b| a.format == b.format);
    Ok(formats)
}

#[tauri::command]
pub async fn tool_get_media_input_extensions(app: AppHandle) -> Result<Vec<String>, String> {
    let ffmpeg = utils::get_ffmpeg_path(&app)?;
    let mut command = Command::new(ffmpeg);
    command
        .args(["-hide_banner", "-demuxers"])
        .kill_on_drop(true);
    #[cfg(target_os = "windows")]
    command.creation_flags(crate::commands::CREATE_NO_WINDOW);
    let output = command
        .output()
        .await
        .map_err(|error| format!("err_run_ffmpeg:{error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_owned());
    }
    let listing = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut extensions = listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let flags = fields.next()?;
            if !flags.contains('D') {
                return None;
            }
            fields.next().map(str::to_owned)
        })
        .flat_map(|names| names.split(',').map(input_extensions).collect::<Vec<_>>())
        .flatten()
        .filter(|extension| {
            !extension.is_empty() && extension.chars().all(|c| c.is_ascii_alphanumeric())
        })
        .collect::<Vec<_>>();
    extensions.sort();
    extensions.dedup();
    Ok(extensions)
}

#[tauri::command]
pub async fn tool_scan_media_directory(directory: String) -> Result<Vec<String>, String> {
    let root = PathBuf::from(directory);
    if !root.is_dir() {
        return Err("err_media_directory_not_found".into());
    }

    let mut pending = vec![root];
    let mut files = Vec::new();
    while let Some(current) = pending.pop() {
        let mut entries = tokio::fs::read_dir(&current)
            .await
            .map_err(|error| format!("err_read_media_directory:{error}"))?;
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|error| format!("err_read_media_directory:{error}"))?
        {
            let file_type = entry
                .file_type()
                .await
                .map_err(|error| format!("err_read_media_file:{error}"))?;
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                let path = entry.path();
                let extension = path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default()
                    .to_ascii_lowercase();
                if !extension.is_empty() {
                    files.push(path.to_string_lossy().into_owned());
                }
            }
        }
    }
    files.sort();
    Ok(files)
}

fn build_output_path(
    input: &Path,
    output_directory: Option<&str>,
    extension: &str,
) -> Result<PathBuf, String> {
    let stem = input
        .file_stem()
        .ok_or_else(|| "err_media_invalid_filename".to_string())?;
    let directory = output_directory
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
        .or_else(|| input.parent().map(Path::to_path_buf))
        .ok_or_else(|| "err_media_missing_output_directory".to_string())?;
    Ok(directory.join(stem).with_extension(extension))
}

fn error_summary(stderr: &[u8]) -> String {
    let message = String::from_utf8_lossy(stderr);
    let mut lines = message
        .lines()
        .filter(|line| !line.trim().is_empty())
        .rev()
        .take(5)
        .collect::<Vec<_>>();
    if lines.is_empty() {
        return "ffmpeg failed".into();
    }
    lines.reverse();
    lines.join(" | ").chars().take(500).collect()
}

#[tauri::command]
pub async fn tool_convert_media_files(
    app: AppHandle,
    run_id: String,
    files: Vec<String>,
    output_format: String,
    output_directory: Option<String>,
    mode: String,
) -> Result<Vec<MediaConversionResult>, String> {
    if files.is_empty() {
        return Err("err_media_no_files".into());
    }
    if !["copy", "encode"].contains(&mode.as_str()) {
        return Err("err_media_invalid_mode".into());
    }
    if output_format.is_empty()
        || !output_format
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return Err("err_media_invalid_format".into());
    }

    let ffmpeg = utils::get_ffmpeg_path(&app)?;
    if let Some(directory) = output_directory
        .as_deref()
        .filter(|path| !path.trim().is_empty())
    {
        tokio::fs::create_dir_all(directory)
            .await
            .map_err(|error| format!("err_create_media_output_directory:{error}"))?;
    }

    let total = files.len();
    let run_suffix = run_id
        .chars()
        .filter(|character| character.is_ascii_alphanumeric())
        .take(32)
        .collect::<String>();
    let mut results = Vec::with_capacity(total);
    for (index, path) in files.into_iter().enumerate() {
        let input = PathBuf::from(&path);
        let filename = input
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.clone());
        let output = match build_output_path(
            &input,
            output_directory.as_deref(),
            output_extension(&output_format),
        ) {
            Ok(output) => output,
            Err(error) => {
                results.push(MediaConversionResult {
                    input: path,
                    output: None,
                    status: "failed".into(),
                    error: Some(error),
                });
                continue;
            }
        };
        let input_abs = tokio::fs::canonicalize(&input).await.ok();
        let output_abs = if output.exists() {
            tokio::fs::canonicalize(&output).await.ok()
        } else {
            None
        };
        if input_abs.is_some() && input_abs == output_abs {
            results.push(MediaConversionResult {
                input: path,
                output: Some(output.to_string_lossy().into_owned()),
                status: "skipped".into(),
                error: Some("err_media_same_input_output".into()),
            });
            continue;
        }
        if tokio::fs::try_exists(&output).await.unwrap_or(false) {
            results.push(MediaConversionResult {
                input: path,
                output: Some(output.to_string_lossy().into_owned()),
                status: "skipped".into(),
                error: Some("err_media_output_exists".into()),
            });
            continue;
        }

        let progress = MediaConversionProgress {
            run_id: run_id.clone(),
            current: index + 1,
            total,
            file: filename.clone(),
            status: "processing".into(),
            output: Some(output.to_string_lossy().into_owned()),
            error: None,
        };
        let _ = app.emit("media-convert-progress", progress);

        let mut command = Command::new(&ffmpeg);
        command.kill_on_drop(true);
        #[cfg(target_os = "windows")]
        command.creation_flags(crate::commands::CREATE_NO_WINDOW);
        let temp_file_name = format!(
            "{}.{}.{}.{}.partial",
            output
                .file_name()
                .map(|name| name.to_string_lossy())
                .unwrap_or_default(),
            run_suffix,
            std::process::id(),
            index
        );
        let temp_output = output.with_file_name(temp_file_name);
        command.args(["-hide_banner", "-nostdin", "-n", "-i"]);
        command.arg(&input);
        command.args([
            "-map",
            "0:v?",
            "-map",
            "0:a?",
            "-map_metadata",
            "0",
            "-map_chapters",
            "0",
        ]);
        if mode == "copy" {
            command.args(["-c", "copy"]);
        }
        command.arg("-f").arg(&output_format);
        command.arg(&temp_output);
        let output_result = command
            .output()
            .await
            .map_err(|error| format!("err_run_ffmpeg:{error}"))?;
        let (status, error) = if output_result.status.success() {
            match tokio::fs::rename(&temp_output, &output).await {
                Ok(()) => ("completed", None),
                Err(error) => {
                    let _ = tokio::fs::remove_file(&temp_output).await;
                    ("failed", Some(format!("err_move_converted_file:{error}")))
                }
            }
        } else {
            let _ = tokio::fs::remove_file(&temp_output).await;
            ("failed", Some(error_summary(&output_result.stderr)))
        };
        let output_path = Some(output.to_string_lossy().into_owned());
        let result = MediaConversionResult {
            input: path,
            output: output_path.clone(),
            status: status.into(),
            error: error.clone(),
        };
        let _ = app.emit(
            "media-convert-progress",
            MediaConversionProgress {
                run_id: run_id.clone(),
                current: index + 1,
                total,
                file: filename,
                status: status.into(),
                output: output_path,
                error,
            },
        );
        results.push(result);
    }
    Ok(results)
}

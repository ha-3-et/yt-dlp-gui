//! 视频缩略图查询与保存。

use crate::commands::support::{self, build_http_client};
use crate::db::{self, tool_tasks};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::process::Stdio;
use tauri::{AppHandle, Manager};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use super::runner::run_ytdlp_tool;

/// 轻量获取视频封面列表（跳过格式检查，速度更快）
#[tauri::command]
pub async fn tool_fetch_thumbnails(
    app: AppHandle,
    run_id: String,
    url: String,
    cookie_file: Option<String>,
    cookie_browser: Option<String>,
    proxy: Option<String>,
) -> Result<Value, String> {
    let mut seen_urls = HashSet::new();
    let urls = url
        .lines()
        .map(str::trim)
        .filter(|value| !value.is_empty() && seen_urls.insert((*value).to_owned()))
        .map(str::to_owned)
        .collect::<Vec<_>>();
    if urls.is_empty() {
        return Err("err_invalid_url".into());
    }

    let mut items = Vec::new();
    let mut errors = Vec::new();
    let mut expected_count = 0_u64;
    for source_url in urls {
        let state = {
            let db = app.state::<db::DatabaseState>();
            tool_tasks::get_state(&db, "thumbnail")?
        };
        if !state
            .as_ref()
            .is_some_and(|task| task.run_id == run_id && task.status == "running")
        {
            return Err("err_tool_task_cancelled".into());
        }

        let info = fetch_thumbnail_metadata(
            &app,
            &run_id,
            &source_url,
            &["--no-check-formats"],
            cookie_file.as_deref(),
            cookie_browser.as_deref(),
            proxy.as_deref(),
        )
        .await;

        match info {
            Ok(info) => {
                if let Some(entries) = info
                    .get("entries")
                    .and_then(Value::as_array)
                    .cloned()
                {
                    let source_start = items.len();
                    let playlist_total = info
                        .get("playlist_count")
                        .and_then(Value::as_u64)
                        .or_else(|| info.get("n_entries").and_then(Value::as_u64))
                        .unwrap_or(entries.len() as u64);
                    expected_count = expected_count.saturating_add(playlist_total);
                    let playlist_title = info
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned();
                    for entry in entries.iter().filter(|entry| !entry.is_null()) {
                        items.push(thumbnail_video_result(
                            entry,
                            Some(&playlist_title),
                            info.get("id").and_then(Value::as_str),
                            entry.get("playlist_index"),
                            &source_url,
                        ));
                    }

                    // Full playlist extraction can omit entries whose video metadata
                    // cannot be resolved. Retry as a flat listing to recover IDs and
                    // use thumbnail URL fallbacks for any recovered entries.
                    if (items.len() - source_start) < playlist_total as usize {
                        if let Ok(flat_info) = fetch_thumbnail_metadata(
                            &app,
                            &run_id,
                            &source_url,
                            &["--no-check-formats", "--flat-playlist"],
                            cookie_file.as_deref(),
                            cookie_browser.as_deref(),
                            proxy.as_deref(),
                        )
                        .await
                        {
                            if let Some(flat_entries) =
                                flat_info.get("entries").and_then(Value::as_array)
                            {
                                let flat_total = flat_info
                                    .get("playlist_count")
                                    .and_then(Value::as_u64)
                                    .or_else(|| {
                                        flat_info.get("n_entries").and_then(Value::as_u64)
                                    })
                                    .unwrap_or(flat_entries.len() as u64);
                                expected_count = expected_count
                                    .saturating_sub(playlist_total)
                                    .saturating_add(playlist_total.max(flat_total));

                                let mut known_ids = HashSet::new();
                                let mut known_indices = HashSet::new();
                                for item in &items[source_start..] {
                                    if let Some(id) = item.get("id").and_then(Value::as_str) {
                                        known_ids.insert(id.to_owned());
                                    }
                                    if let Some(index) =
                                        item.get("playlistIndex").and_then(Value::as_u64)
                                    {
                                        known_indices.insert(index);
                                    }
                                }
                                for entry in flat_entries.iter().filter(|entry| !entry.is_null()) {
                                    let item = thumbnail_video_result(
                                        entry,
                                        Some(&playlist_title),
                                        flat_info.get("id").and_then(Value::as_str),
                                        entry.get("playlist_index"),
                                        &source_url,
                                    );
                                    let id = item
                                        .get("id")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default();
                                    let index =
                                        item.get("playlistIndex").and_then(Value::as_u64);
                                    let duplicate = index.map_or_else(
                                        || !id.is_empty() && known_ids.contains(id),
                                        |index| known_indices.contains(&index),
                                    );
                                    if !duplicate {
                                        if !id.is_empty() {
                                            known_ids.insert(id.to_owned());
                                        }
                                        if let Some(index) = index {
                                            known_indices.insert(index);
                                        }
                                        items.push(item);
                                    }
                                }
                            }
                        }
                    }
                } else {
                    expected_count = expected_count.saturating_add(1);
                    items.push(thumbnail_video_result(&info, None, None, None, &source_url));
                }
            }
            Err(error) => {
                expected_count = expected_count.saturating_add(1);
                let state = {
                    let db = app.state::<db::DatabaseState>();
                    tool_tasks::get_state(&db, "thumbnail")?
                };
                if !state
                    .as_ref()
                    .is_some_and(|task| task.run_id == run_id && task.status == "running")
                {
                    return Err("err_tool_task_cancelled".into());
                }
                errors.push(json!({ "url": source_url, "error": error }));
            }
        }
    }

    Ok(json!({ "items": items, "errors": errors, "expectedCount": expected_count }))
}

async fn fetch_thumbnail_metadata(
    app: &AppHandle,
    run_id: &str,
    url: &str,
    extra_args: &[&str],
    cookie_file: Option<&str>,
    cookie_browser: Option<&str>,
    proxy: Option<&str>,
) -> Result<Value, String> {
    let result = support::run_ytdlp_json_tracked(
        app,
        Some(run_id),
        url,
        extra_args,
        cookie_file,
        cookie_browser,
        proxy,
    )
    .await;
    match result {
        Err(error)
            if cookie_browser.is_some()
                && error.to_ascii_lowercase().contains("dpapi")
                && error.to_ascii_lowercase().contains("decrypt") =>
        {
            // Public video metadata usually does not need browser cookies.
            // If yt-dlp cannot decrypt the selected browser's cookie store,
            // retry without cookies instead of failing the whole URL.
            support::run_ytdlp_json_tracked(
                app,
                Some(run_id),
                url,
                extra_args,
                None,
                None,
                proxy,
            )
            .await
        }
        result => result,
    }
}

fn thumbnail_video_result(
    info: &Value,
    playlist_title: Option<&str>,
    playlist_id: Option<&str>,
    playlist_index: Option<&Value>,
    source_url: &str,
) -> Value {
    json!({
        "id": info.get("id").and_then(Value::as_str).unwrap_or_default(),
        "title": info.get("title").and_then(Value::as_str).unwrap_or_default(),
        "thumbnail": info.get("thumbnail").and_then(Value::as_str).unwrap_or_default(),
        "thumbnails": info
            .get("thumbnails")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        "playlistTitle": playlist_title.unwrap_or_default(),
        "playlistId": playlist_id.unwrap_or_default(),
        "playlistIndex": playlist_index.and_then(Value::as_u64),
        "sourceUrl": source_url,
        "uploader": info
            .get("uploader")
            .or_else(|| info.get("channel"))
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "uploadDate": info
            .get("upload_date")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        "duration": info.get("duration").and_then(Value::as_f64),
        "videoHeight": info.get("height").and_then(Value::as_u64),
        "videoResolution": info
            .get("resolution")
            .and_then(Value::as_str)
            .unwrap_or_default(),
    })
}

/// 将指定 URL 的图片下载到指定文件路径（另存为）
#[tauri::command]
pub async fn tool_save_thumbnail(
    app: AppHandle,
    url: String,
    file_path: String,
    proxy: Option<String>,
) -> Result<(), String> {
    let client = build_http_client(proxy.as_deref())?;

    let response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("err_download_thumbnail:{}", e))?;

    if !response.status().is_success() {
        return Err(format!("err_download_thumbnail:HTTP {}", response.status()));
    }

    let source_bytes = response
        .bytes()
        .await
        .map_err(|e| format!("err_read_thumbnail_data:{}", e))?;

    let ffmpeg_path = crate::utils::get_ffmpeg_path(&app)?;
    let mut command = Command::new(ffmpeg_path);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            "pipe:0",
            "-frames:v",
            "1",
            "-f",
            "image2pipe",
            "-vcodec",
            "png",
            "pipe:1",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(target_os = "windows")]
    command.creation_flags(crate::commands::CREATE_NO_WINDOW);

    let mut child = command
        .spawn()
        .map_err(|e| format!("err_run_ffmpeg_thumbnail:{}", e))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "err_ffmpeg_thumbnail_stdin".to_string())?;
    stdin
        .write_all(&source_bytes)
        .await
        .map_err(|e| format!("err_write_ffmpeg_thumbnail:{}", e))?;
    drop(stdin);
    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("err_wait_ffmpeg_thumbnail:{}", e))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(format!("err_convert_thumbnail_to_png:{}", detail));
    }
    if output.stdout.is_empty() {
        return Err("err_convert_thumbnail_to_png:empty_output".into());
    }

    if let Some(parent) = std::path::Path::new(&file_path).parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("err_create_dir:{}", e))?;
    }

    tokio::fs::write(&file_path, &output.stdout)
        .await
        .map_err(|e| format!("err_save_file:{}", e))?;

    Ok(())
}

/// 下载视频封面图
#[tauri::command]
pub async fn tool_download_thumbnail(
    app: AppHandle,
    url: String,
    download_dir: String,
    cookie_file: Option<String>,
    cookie_browser: Option<String>,
    proxy: Option<String>,
) -> Result<String, String> {
    run_ytdlp_tool(
        &app,
        None,
        &url,
        &download_dir,
        vec![
            "--write-thumbnail".to_string(),
            "--convert-thumbnails".to_string(),
            "jpg".to_string(),
        ],
        cookie_file.as_deref(),
        cookie_browser.as_deref(),
        proxy.as_deref(),
    )
    .await
}

//! Screen OCR service ("Capture Text from Screen").
//!
//! Provides on-device, privacy-preserving interactive screen snippet capture and
//! local text recognition. Extracted text is checked against Asyar's Layer 2
//! secret redaction engine before being written to the system clipboard and
//! signaled via an Asyar native HUD notification.

use crate::error::AppError;
use crate::AppState;
use std::path::PathBuf;
use tauri::Manager;

#[cfg(target_os = "linux")]
pub mod linux;
#[cfg(target_os = "macos")]
pub mod macos;
#[cfg(target_os = "windows")]
pub mod windows;

/// Captures a screen snippet interactively, saving it to a temporary image file.
///
/// Returns `Ok(None)` if the user cancelled the capture (e.g. pressed Escape).
pub fn capture_screen_snippet() -> Result<Option<PathBuf>, AppError> {
    #[cfg(target_os = "macos")]
    {
        macos::capture_screen_snippet()
    }
    #[cfg(target_os = "windows")]
    {
        windows::capture_screen_snippet()
    }
    #[cfg(target_os = "linux")]
    {
        linux::capture_screen_snippet()
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        Err(AppError::Platform(
            "Screen capture is not supported on this platform".into(),
        ))
    }
}

/// Runs local on-device OCR on the provided image bytes.
pub fn recognize_text(image_bytes: &[u8]) -> Result<String, AppError> {
    #[cfg(target_os = "macos")]
    {
        macos::recognize_text(image_bytes)
    }
    #[cfg(target_os = "windows")]
    {
        windows::recognize_text(image_bytes)
    }
    #[cfg(target_os = "linux")]
    {
        linux::recognize_text(image_bytes)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    {
        let _ = image_bytes;
        Err(AppError::Platform(
            "OCR is not supported on this platform".into(),
        ))
    }
}

/// Result of evaluating OCR text for clipboard copy and HUD messaging.
#[derive(Debug, PartialEq, Eq)]
pub enum OcrEvaluation {
    /// No text was found (or only whitespace). Contains the HUD notification message.
    Empty { hud_message: String },
    /// Text was recognized, redacted, and prepared for clipboard.
    Success {
        redacted_text: String,
        hud_message: String,
    },
}

/// Default HUD message when Screen OCR finds no text.
pub const EMPTY_SCREEN_OCR_HUD_MESSAGE: &str = "No text found in selected area";

/// Default HUD message when Clipboard Image OCR finds no text.
pub const EMPTY_IMAGE_OCR_HUD_MESSAGE: &str = "No text found in image";

/// Evaluates raw OCR text: applies secret redaction and determines HUD notification copy.
pub fn evaluate_ocr_text(raw_text: &str) -> OcrEvaluation {
    evaluate_ocr_text_with_empty_hud(raw_text, EMPTY_SCREEN_OCR_HUD_MESSAGE)
}

/// Evaluates raw OCR text with a caller-specific empty-text HUD notification message.
pub fn evaluate_ocr_text_with_empty_hud(raw_text: &str, empty_hud_message: &str) -> OcrEvaluation {
    let trimmed = raw_text.trim();
    if trimmed.is_empty() {
        return OcrEvaluation::Empty {
            hud_message: empty_hud_message.to_string(),
        };
    }

    let redaction = crate::secret_detection::redact(trimmed);
    let content = redaction.content;

    let hud_message = if !content.contains('\n') && content.chars().count() <= 30 {
        format!("Text copied: \"{}\"", content)
    } else {
        "Text copied to clipboard".to_string()
    };

    OcrEvaluation::Success {
        redacted_text: content,
        hud_message,
    }
}

/// Orchestrates full Screen OCR flow:
/// 1. Hides/parks launcher window
/// 2. Spawns interactive screen capture overlay
/// 3. Runs local OCR on the snippet
/// 4. Cleans up temp files immediately
/// 5. Writes text to clipboard & shows HUD notification
pub async fn capture_screen_text(
    app_handle: &tauri::AppHandle,
    _state: &tauri::State<'_, AppState>,
) -> Result<Option<String>, AppError> {
    // 1. Hide/park launcher on main thread so it is not captured in the screen snippet
    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let app_clone = app_handle.clone();
    app_handle
        .run_on_main_thread(move || {
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let app_state = app_clone.state::<AppState>();
                let _ = crate::commands::app::hide(app_clone.clone(), app_state);
                let _ = tx.send(());
            }));
        })
        .map_err(|e| AppError::Platform(format!("failed to dispatch hide to main thread: {e}")))?;
    let _ = rx.await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;

    // 2. Interactive screen selection
    let snippet_opt = tauri::async_runtime::spawn_blocking(capture_screen_snippet)
        .await
        .map_err(|e| AppError::Platform(format!("screen capture task failed: {e}")))?;

    let temp_path = match snippet_opt? {
        Some(path) => path,
        None => {
            // User cancelled snippet capture (e.g. pressed Escape)
            return Ok(None);
        }
    };

    // 3. Read image bytes and clean up file immediately
    let image_bytes = std::fs::read(&temp_path).map_err(|e| {
        let _ = std::fs::remove_file(&temp_path);
        AppError::Io(e)
    })?;
    let _ = std::fs::remove_file(&temp_path);

    // 4. Local on-device OCR + secret redaction + clipboard write + HUD
    recognize_and_copy_text(app_handle, image_bytes, EMPTY_SCREEN_OCR_HUD_MESSAGE).await
}

/// Runs local OCR on `image_bytes`, applies secret redaction, writes any
/// recognized text to the system clipboard, and displays the native HUD
/// notification. Shared by Screen OCR and Clipboard Image OCR.
async fn recognize_and_copy_text(
    app_handle: &tauri::AppHandle,
    image_bytes: Vec<u8>,
    empty_hud_message: &'static str,
) -> Result<Option<String>, AppError> {
    let raw_text = tauri::async_runtime::spawn_blocking(move || recognize_text(&image_bytes))
        .await
        .map_err(|e| AppError::Platform(format!("OCR task failed: {e}")))??;

    match evaluate_ocr_text_with_empty_hud(&raw_text, empty_hud_message) {
        OcrEvaluation::Empty { hud_message } => {
            let app_clone = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = crate::hud_window::service::show(&app_clone, hud_message, 2000, false);
                }));
            });
            Ok(None)
        }
        OcrEvaluation::Success {
            redacted_text,
            hud_message,
        } => {
            // Record secret detection metrics if state is available
            if let Some(sec_state) =
                app_handle.try_state::<crate::secret_detection::SecretDetectionState>()
            {
                let redaction = crate::secret_detection::redact(&redacted_text);
                sec_state.record(&redaction.kinds);
            }

            // Write to system clipboard
            let mut clipboard = arboard::Clipboard::new()
                .map_err(|e| AppError::Platform(format!("failed to open clipboard: {e}")))?;
            clipboard
                .set_text(&redacted_text)
                .map_err(|e| AppError::Platform(format!("failed to write clipboard: {e}")))?;

            // Show HUD notification on main thread
            let app_clone = app_handle.clone();
            let _ = app_handle.run_on_main_thread(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _ = crate::hud_window::service::show(&app_clone, hud_message, 2000, false);
                }));
            });

            Ok(Some(redacted_text))
        }
    }
}

/// Resolves a clipboard history item by `id` from SQLite, verifies that it is an
/// `image` item whose path lives inside the app's `clipboard_cache` directory,
/// and confirms the file exists on disk.
pub fn resolve_clipboard_image_path(
    conn: &rusqlite::Connection,
    app_data_dir: &std::path::Path,
    id: &str,
    master_key: &[u8; 32],
) -> Result<PathBuf, AppError> {
    let item = crate::storage::clipboard::get_item(conn, id, master_key)?
        .ok_or_else(|| AppError::NotFound(format!("clipboard item not found: {id}")))?;

    if item.item_type != "image" {
        return Err(AppError::Validation(format!(
            "clipboard item {id} is not an image (found {})",
            item.item_type
        )));
    }

    let raw_path = item
        .content
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            AppError::Validation(format!("clipboard image item {id} has no file path"))
        })?;

    let path = PathBuf::from(raw_path);
    if !crate::clipboard_cache::is_cached_image(app_data_dir, &path) {
        return Err(AppError::Validation(format!(
            "clipboard item {id} path is outside clipboard_cache"
        )));
    }

    if !path.is_file() {
        return Err(AppError::NotFound(format!(
            "cached clipboard image file not found: {}",
            path.display()
        )));
    }

    Ok(path)
}

/// Runs local OCR on a cached clipboard history image resolved by `id` from the
/// database, copies any recognized text to the clipboard, and shows the HUD.
pub async fn ocr_clipboard_item_text(
    app_handle: &tauri::AppHandle,
    store: &tauri::State<'_, crate::storage::DataStore>,
    keystore: &tauri::State<'_, crate::crypto::keystore::KeystoreState>,
    id: &str,
) -> Result<Option<String>, AppError> {
    let app_data_dir = app_handle
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Platform(format!("app data dir unavailable: {e}")))?;
    let image_path = {
        let conn = store.conn()?;
        resolve_clipboard_image_path(&conn, &app_data_dir, id, keystore.master_key())?
    };
    let image_bytes = std::fs::read(&image_path)?;
    recognize_and_copy_text(app_handle, image_bytes, EMPTY_IMAGE_OCR_HUD_MESSAGE).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evaluate_empty_or_whitespace_text() {
        let res = evaluate_ocr_text("");
        assert!(matches!(res, OcrEvaluation::Empty { .. }));
        if let OcrEvaluation::Empty { hud_message } = res {
            assert_eq!(hud_message, "No text found in selected area");
        }

        let res2 = evaluate_ocr_text("   \n\t  ");
        assert!(matches!(res2, OcrEvaluation::Empty { .. }));

        let res_img = evaluate_ocr_text_with_empty_hud("   ", EMPTY_IMAGE_OCR_HUD_MESSAGE);
        assert_eq!(
            res_img,
            OcrEvaluation::Empty {
                hud_message: "No text found in image".to_string(),
            }
        );
    }

    #[test]
    fn test_evaluate_plain_text_short_preview() {
        let res = evaluate_ocr_text("Hello World");
        match res {
            OcrEvaluation::Success {
                redacted_text,
                hud_message,
            } => {
                assert_eq!(redacted_text, "Hello World");
                assert_eq!(hud_message, "Text copied: \"Hello World\"");
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn test_evaluate_multiline_text_standard_hud() {
        let res = evaluate_ocr_text("Line 1\nLine 2\nLine 3");
        match res {
            OcrEvaluation::Success {
                redacted_text,
                hud_message,
            } => {
                assert_eq!(redacted_text, "Line 1\nLine 2\nLine 3");
                assert_eq!(hud_message, "Text copied to clipboard");
            }
            _ => panic!("expected Success"),
        }
    }

    #[test]
    fn test_evaluate_redacts_secrets_in_text() {
        let res = evaluate_ocr_text("API key: AKIAIOSFODNN7EXAMPLE done");
        match res {
            OcrEvaluation::Success {
                redacted_text,
                hud_message,
            } => {
                assert!(redacted_text.contains("[redacted: aws_access_key]"));
                assert!(!redacted_text.contains("AKIAIOSFODNN7EXAMPLE"));
                assert!(hud_message.contains("Text copied"));
            }
            _ => panic!("expected Success"),
        }
    }

    fn temp_app_data_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "asyar_ocr_clip_{tag}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn test_db_and_key() -> (rusqlite::Connection, [u8; 32]) {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        crate::storage::clipboard::init_table(&conn).unwrap();
        (conn, [7u8; 32])
    }

    fn make_clipboard_item(
        id: &str,
        item_type: &str,
        content: Option<String>,
    ) -> crate::storage::clipboard::ClipboardItem {
        crate::storage::clipboard::ClipboardItem {
            id: id.to_string(),
            item_type: item_type.to_string(),
            content,
            preview: Some("preview".to_string()),
            created_at: 1_000.0,
            favorite: false,
            metadata: None,
            source_app: None,
            redacted_kinds: None,
        }
    }

    #[test]
    fn test_resolve_clipboard_image_path_success() {
        let (conn, key) = test_db_and_key();
        let root = temp_app_data_dir("ok");
        let cached = crate::clipboard_cache::cached_image_path(&root, "img-1").unwrap();
        std::fs::create_dir_all(cached.parent().unwrap()).unwrap();
        std::fs::write(&cached, b"png-bytes").unwrap();

        let item = make_clipboard_item(
            "img-1",
            "image",
            Some(cached.to_string_lossy().into_owned()),
        );
        crate::storage::clipboard::add_item(&conn, &item, &key).unwrap();

        let resolved = resolve_clipboard_image_path(&conn, &root, "img-1", &key).unwrap();
        assert_eq!(resolved, cached);

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_resolve_clipboard_image_path_not_found() {
        let (conn, key) = test_db_and_key();
        let root = temp_app_data_dir("not_found");

        let err = resolve_clipboard_image_path(&conn, &root, "missing-id", &key)
            .expect_err("missing item id should fail");
        assert!(
            matches!(err, AppError::NotFound(_)),
            "expected AppError::NotFound, got {err:?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_resolve_clipboard_image_path_not_an_image() {
        let (conn, key) = test_db_and_key();
        let root = temp_app_data_dir("not_image");

        let item = make_clipboard_item("txt-1", "text", Some("hello world".to_string()));
        crate::storage::clipboard::add_item(&conn, &item, &key).unwrap();

        let err = resolve_clipboard_image_path(&conn, &root, "txt-1", &key)
            .expect_err("non-image item should fail");
        assert!(
            matches!(err, AppError::Validation(_)),
            "expected AppError::Validation, got {err:?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_resolve_clipboard_image_path_rejects_outside_cache() {
        let (conn, key) = test_db_and_key();
        let root = temp_app_data_dir("outside_cache");
        let outside = root.join("other_dir/img-1.png");
        std::fs::create_dir_all(outside.parent().unwrap()).unwrap();
        std::fs::write(&outside, b"png-bytes").unwrap();

        let item = make_clipboard_item(
            "img-1",
            "image",
            Some(outside.to_string_lossy().into_owned()),
        );
        crate::storage::clipboard::add_item(&conn, &item, &key).unwrap();

        let err = resolve_clipboard_image_path(&conn, &root, "img-1", &key)
            .expect_err("path outside clipboard_cache should fail");
        assert!(
            matches!(err, AppError::Validation(_)),
            "expected AppError::Validation, got {err:?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_resolve_clipboard_image_path_missing_file() {
        let (conn, key) = test_db_and_key();
        let root = temp_app_data_dir("missing_file");
        let cached = crate::clipboard_cache::cached_image_path(&root, "img-missing").unwrap();

        let item = make_clipboard_item(
            "img-missing",
            "image",
            Some(cached.to_string_lossy().into_owned()),
        );
        crate::storage::clipboard::add_item(&conn, &item, &key).unwrap();

        let err = resolve_clipboard_image_path(&conn, &root, "img-missing", &key)
            .expect_err("missing cached file on disk should fail");
        assert!(
            matches!(err, AppError::NotFound(_)),
            "expected AppError::NotFound, got {err:?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn test_resolve_clipboard_image_path_empty_or_none_content() {
        let (conn, key) = test_db_and_key();
        let root = temp_app_data_dir("empty_content");

        let none_item = make_clipboard_item("img-none", "image", None);
        crate::storage::clipboard::add_item(&conn, &none_item, &key).unwrap();
        let err_none = resolve_clipboard_image_path(&conn, &root, "img-none", &key)
            .expect_err("image item with None content should fail");
        assert!(
            matches!(err_none, AppError::Validation(_)),
            "expected AppError::Validation, got {err_none:?}"
        );

        let blank_item = make_clipboard_item("img-blank", "image", Some("   \t\n  ".to_string()));
        crate::storage::clipboard::add_item(&conn, &blank_item, &key).unwrap();
        let err_blank = resolve_clipboard_image_path(&conn, &root, "img-blank", &key)
            .expect_err("image item with whitespace-only content should fail");
        assert!(
            matches!(err_blank, AppError::Validation(_)),
            "expected AppError::Validation, got {err_blank:?}"
        );

        std::fs::remove_dir_all(&root).ok();
    }
}

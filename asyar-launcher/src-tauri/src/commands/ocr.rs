//! Tauri command layer for screen capture OCR.

use crate::error::AppError;
use crate::AppState;

#[tauri::command]
pub async fn ocr_capture_screen_text(
    app_handle: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<Option<String>, AppError> {
    crate::ocr::capture_screen_text(&app_handle, &state).await
}

#[tauri::command]
pub async fn ocr_clipboard_item_text(
    id: String,
    app_handle: tauri::AppHandle,
    store: tauri::State<'_, crate::storage::DataStore>,
    keystore: tauri::State<'_, crate::crypto::keystore::KeystoreState>,
) -> Result<Option<String>, AppError> {
    crate::ocr::ocr_clipboard_item_text(&app_handle, &store, &keystore, &id).await
}

//! WebKitGTK refuses `getUserMedia` until the app embedding it turns media
//! capture on, and wry doesn't.

use tauri::WebviewWindow;
use webkit2gtk::{SettingsExt, WebViewExt};

pub fn enable(window: &WebviewWindow) -> tauri::Result<()> {
    window.with_webview(|webview| {
        if let Some(settings) = webview.inner().settings() {
            settings.set_enable_media_stream(true);
        }
    })
}

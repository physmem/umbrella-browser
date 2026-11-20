use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("Windows call failed: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("WebView2 call failed: {0}")]
    WebView2(String),
    #[error("cannot prepare profile folder: {0}")]
    Io(#[from] std::io::Error),
    #[error("WebView2 returned no object")]
    Missing,
}

impl From<webview2_com::Error> for EngineError {
    fn from(e: webview2_com::Error) -> Self {
        match e {
            webview2_com::Error::WindowsError(e) => EngineError::Windows(e),
            other => EngineError::WebView2(other.to_string()),
        }
    }
}

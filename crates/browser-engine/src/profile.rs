use std::fs;
use std::path::PathBuf;

use webview2_com::Microsoft::Web::WebView2::Win32::{
    CreateCoreWebView2EnvironmentWithOptions, ICoreWebView2Environment,
    ICoreWebView2EnvironmentOptions,
};
use webview2_com::{CoreWebView2EnvironmentOptions, CreateCoreWebView2EnvironmentCompletedHandler};
use windows::core::HSTRING;

use crate::EngineError;

/// Turns off WebView2 extras a lean browser does not need. SmartScreen and
/// Safe Browsing stay on: they protect users and cost little.
const BROWSER_ARGS: &str = "--no-first-run --disable-sync --disable-default-apps --disable-features=msEdgeShoppingAssistant";

#[derive(Debug, Clone)]
pub struct ProfileOptions {
    /// One folder per workspace keeps cookies, storage and cache isolated.
    pub user_data_dir: PathBuf,
    pub language: Option<String>,
    /// Extra Chromium switches, e.g. `--disable-gpu`.
    pub extra_args: Vec<String>,
}

/// One WebView2 environment, i.e. one browser process tree and data folder.
#[derive(Clone)]
pub struct Profile {
    pub(crate) env: ICoreWebView2Environment,
}

impl Profile {
    /// Starts creating the environment. `on_ready` runs later from the UI
    /// thread's message loop; this call never blocks or pumps messages.
    pub fn create<F>(options: &ProfileOptions, on_ready: F) -> Result<(), EngineError>
    where
        F: FnOnce(Result<Profile, EngineError>) + 'static,
    {
        fs::create_dir_all(&options.user_data_dir)?;
        let env_options = environment_options(options);
        let data_dir = HSTRING::from(options.user_data_dir.as_os_str());
        let handler =
            CreateCoreWebView2EnvironmentCompletedHandler::create(Box::new(move |result, env| {
                on_ready(
                    result
                        .map_err(EngineError::from)
                        .and_then(|()| env.map(|env| Profile { env }).ok_or(EngineError::Missing)),
                );
                Ok(())
            }));
        // SAFETY: arguments are live for the call; WebView2 AddRefs the
        // options and handler it keeps.
        unsafe {
            CreateCoreWebView2EnvironmentWithOptions(None, &data_dir, &env_options, &handler)?;
        }
        Ok(())
    }
}

/// Chromium reads only the last `--disable-features`, so every feature
/// list is merged into one switch.
fn browser_args(extra: &[String]) -> String {
    const FEATURES: &str = "--disable-features=";
    let mut features: Vec<&str> = Vec::new();
    let mut args: Vec<&str> = Vec::new();
    let all = BROWSER_ARGS
        .split_whitespace()
        .chain(extra.iter().map(String::as_str));
    for arg in all {
        match arg.strip_prefix(FEATURES) {
            Some(list) => features.extend(list.split(',').filter(|f| !f.is_empty())),
            None => args.push(arg),
        }
    }
    let mut line = args.join(" ");
    if !features.is_empty() {
        line.push_str(&format!(" {FEATURES}{}", features.join(",")));
    }
    line
}

fn environment_options(options: &ProfileOptions) -> ICoreWebView2EnvironmentOptions {
    let env_options = CoreWebView2EnvironmentOptions::default();
    // SAFETY: the options object is not shared with WebView2 yet.
    unsafe {
        env_options.set_additional_browser_arguments(browser_args(&options.extra_args));
        // Extensions only load into profiles created with this on.
        env_options.set_are_browser_extensions_enabled(true);
        if let Some(lang) = &options.language {
            env_options.set_language(lang.clone());
        }
    }
    env_options.into()
}

#[cfg(test)]
mod tests {
    #[test]
    fn disabled_features_merge_into_one_switch() {
        let line = super::browser_args(&[
            "--in-process-gpu".to_owned(),
            "--disable-features=AudioServiceOutOfProcess".to_owned(),
        ]);
        assert_eq!(line.matches("--disable-features=").count(), 1);
        assert!(
            line.ends_with("--disable-features=msEdgeShoppingAssistant,AudioServiceOutOfProcess")
        );
        assert!(line.contains("--in-process-gpu"));
    }
}

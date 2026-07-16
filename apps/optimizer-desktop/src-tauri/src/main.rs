use std::env;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use optimizer_host::SecretStore;
use tauri::Manager;

#[cfg(not(windows))]
use optimizer_host::MemorySecretStore;
#[cfg(windows)]
use optimizer_host::WindowsCredentialStore;

fn main() {
    let secrets = platform_secret_store();
    let state = optimizer_desktop::DesktopState::new(secrets);
    optimizer_desktop::attach(tauri::Builder::default(), state)
        .setup(|app| {
            let recent_projects_path = app
                .path()
                .app_local_data_dir()?
                .join("recent-projects.json");
            app.state::<optimizer_desktop::DesktopState>()
                .configure_recent_projects(&recent_projects_path)
                .map_err(|error| {
                    io::Error::other(format!(
                        "failed to configure recent projects at {}: {error}",
                        recent_projects_path.display()
                    ))
                })?;
            let window_config = app
                .config()
                .app
                .windows
                .iter()
                .find(|window| window.label == "main")
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "main window config"))?;
            let data_directory = webview_data_directory(app)?;
            std::fs::create_dir_all(&data_directory).map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!(
                        "failed to create WebView data directory {}: {error}",
                        data_directory.display()
                    ),
                )
            })?;
            tauri::WebviewWindowBuilder::from_config(app.handle(), window_config)
                .map_err(|error| io::Error::other(format!("invalid main window config: {error}")))?
                .data_directory(data_directory)
                .build()
                .map_err(|error| {
                    io::Error::other(format!("failed to create main window: {error}"))
                })?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("Optimizer System desktop runtime failed");
}

fn webview_data_directory<R: tauri::Runtime>(
    _app: &tauri::App<R>,
) -> Result<PathBuf, Box<dyn std::error::Error>> {
    let path = if let Some(override_path) = env::var_os("OPTIMIZER_WEBVIEW_DATA_DIR") {
        PathBuf::from(override_path)
    } else {
        #[cfg(debug_assertions)]
        {
            env::current_exe()?
                .parent()
                .ok_or_else(|| io::Error::other("desktop executable has no parent directory"))?
                .join(".optimizer-webview")
        }
        #[cfg(not(debug_assertions))]
        {
            _app.path().app_local_data_dir()?.join("webview")
        }
    };
    if !path.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "OPTIMIZER_WEBVIEW_DATA_DIR must be absolute",
        )
        .into());
    }
    Ok(path)
}

#[cfg(windows)]
fn platform_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(
        WindowsCredentialStore::new("OptimizerSystem")
            .expect("Windows Credential Manager namespace is valid"),
    )
}

#[cfg(not(windows))]
fn platform_secret_store() -> Arc<dyn SecretStore> {
    Arc::new(MemorySecretStore::default())
}

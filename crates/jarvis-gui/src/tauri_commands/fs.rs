#[cfg(target_os = "linux")]
use std::fs::metadata;
#[cfg(target_os = "linux")]
use std::path::PathBuf;

use std::process::Command;

// taken from https://github.com/tauri-apps/tauri/issues/4062#issuecomment-1338048169
#[tauri::command]
pub fn show_in_folder(path: String) -> Result<(), String> {
  #[cfg(target_os = "windows")]
  {
    Command::new("explorer")
        .args(["/select,", &path]) // The comma after select is not a typo
        .spawn().map_err(|e| e.to_string())?;
  }

  #[cfg(target_os = "linux")]
  {
    if path.contains(",") {
      // see https://gitlab.freedesktop.org/dbus/dbus/-/issues/76
      let new_path = match metadata(&path).map_err(|e| e.to_string())?.is_dir() {
        true => path,
        false => {
          let mut path2 = PathBuf::from(path);
          path2.pop();
          path2.to_string_lossy().to_string()
        }
      };
      Command::new("xdg-open")
          .arg(&new_path)
          .spawn().map_err(|e| e.to_string())?;
    } else {
      let url = reqwest::Url::from_file_path(&path).map_err(|_| "Нужен полный путь к файлу".to_string())?;
      Command::new("dbus-send")
          .args(["--session", "--dest=org.freedesktop.FileManager1", "--type=method_call",
                "/org/freedesktop/FileManager1", "org.freedesktop.FileManager1.ShowItems",
                format!("array:string:{url}").as_str(), "string:"])
          .spawn().map_err(|e| e.to_string())?;
    }
  }

  #[cfg(target_os = "macos")]
  {
    Command::new("open")
        .args(["-R", &path])
        .spawn().map_err(|e| e.to_string())?;
  }
  Ok(())
}

//! Hyperlink click-to-open

/// Open a URL in the default browser
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", url])
            .spawn();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(url)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(url)
            .spawn();
    }
}

/// Open a file path
pub fn open_path(path: &str, line: Option<u32>, column: Option<u32>) {
    let path_with_location = if let Some(line) = line {
        if let Some(col) = column {
            format!("{}:{}:{}", path, line, col)
        } else {
            format!("{}:{}", path, line)
        }
    } else {
        path.to_string()
    };

    // Try VS Code
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("code")
            .args(["-g", &path_with_location])
            .spawn();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("code")
            .args(["-g", &path_with_location])
            .spawn();
    }
}

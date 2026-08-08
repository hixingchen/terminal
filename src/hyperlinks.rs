//! Hyperlink click-to-open

/// Sanitize a string for safe use with cmd /C start.
/// Removes shell metacharacters that could enable command injection.
fn sanitize_for_shell(s: &str) -> String {
    s.chars()
        .filter(|c| !matches!(c, '&' | '|' | ';' | '>' | '<' | '^' | '`' | '$' | '(' | ')' | '!' | '%'))
        .collect()
}

/// Open a URL in the default browser
pub fn open_url(url: &str) {
    // Only allow http/https URLs to prevent scheme-based attacks
    if !url.starts_with("http://") && !url.starts_with("https://") {
        return;
    }
    let safe_url = sanitize_for_shell(url);
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("cmd")
            .args(["/C", "start", &safe_url])
            .spawn();
    }
    #[cfg(unix)]
    {
        let _ = std::process::Command::new("xdg-open")
            .arg(&safe_url)
            .spawn();
    }
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("open")
            .arg(&safe_url)
            .spawn();
    }
}

/// Open a file path in VS Code.
/// Uses direct process spawn (not shell), safe from injection.
pub fn open_path(path: &str, line: Option<u32>, column: Option<u32>) {
    // Basic path validation: reject paths containing shell metacharacters
    if path.contains('&') || path.contains('|') || path.contains(';')
        || path.contains('>') || path.contains('<') || path.contains('^')
        || path.contains('`') || path.contains('$')
    {
        return;
    }

    let path_with_location = if let Some(line) = line {
        if let Some(col) = column {
            format!("{}:{}:{}", path, line, col)
        } else {
            format!("{}:{}", path, line)
        }
    } else {
        path.to_string()
    };

    // Try VS Code — direct spawn, not through shell
    let _ = std::process::Command::new("code")
        .args(["-g", &path_with_location])
        .spawn();
}

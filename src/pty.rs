//! PTY (Pseudo-Terminal) management
//!
//! Creates a real PTY and spawns a shell process with full support.

use anyhow::{Context, Result};
use portable_pty::{CommandBuilder, PtyPair, PtySize, native_pty_system};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

/// PTY process wrapper
#[allow(dead_code)]
pub struct PtyProcess {
    /// PTY pair
    pty: Arc<Mutex<PtyPair>>,
    /// Child process
    child: Arc<Mutex<Box<dyn portable_pty::Child + Send + Sync>>>,
    /// Writer to PTY
    writer: Arc<Mutex<Box<dyn Write + Send>>>,
    /// Reader from PTY
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
}

impl PtyProcess {
    /// Create a new PTY process with a shell and optional working directory
    pub fn new_with_cwd(cols: u16, rows: u16, working_dir: Option<String>) -> Result<Self> {
        let pty_system = native_pty_system();

        // Create PTY pair
        let pty_pair = pty_system
            .openpty(PtySize {
                rows,
                cols,
                pixel_width: 0,
                pixel_height: 0,
            })
            .context("Failed to create PTY")?;

        // Get shell command
        let shell = get_shell();
        let args = get_shell_args(&shell);

        // Build command
        let mut cmd = CommandBuilder::new(&shell);
        for arg in &args {
            cmd.arg(arg);
        }
        // FIX 8: Use restored working directory if available
        let cwd = working_dir.unwrap_or_else(|| get_cwd());
        cmd.cwd(cwd);

        // Set environment variables
        cmd.env("TERM", "xterm-256color");
        cmd.env("COLORTERM", "truecolor");
        cmd.env("TERM_PROGRAM", "terminal");
        cmd.env("TERM_PROGRAM_VERSION", "0.1.0");

        // Spawn child process
        let child = pty_pair
            .slave
            .spawn_command(cmd)
            .context("Failed to spawn shell process")?;

        // Get reader and writer
        let reader = pty_pair.master.try_clone_reader().context("Failed to clone reader")?;
        let writer = pty_pair.master.take_writer().context("Failed to take writer")?;

        Ok(Self {
            pty: Arc::new(Mutex::new(pty_pair)),
            child: Arc::new(Mutex::new(child)),
            writer: Arc::new(Mutex::new(writer)),
            reader: Arc::new(Mutex::new(reader)),
        })
    }

    /// Write data to PTY (send to shell)
    pub fn write(&self, data: &[u8]) -> Result<()> {
        let mut writer = self.writer.lock().unwrap_or_else(|e| e.into_inner());
        writer.write_all(data)?;
        writer.flush()?;
        Ok(())
    }

    /// Get a reader for PTY output
    pub fn reader(&self) -> PtyReader {
        PtyReader {
            reader: self.reader.clone(),
        }
    }

    /// Resize the PTY
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        let pty = self.pty.lock().unwrap_or_else(|e| e.into_inner());
        pty.master.resize(PtySize {
            rows,
            cols,
            pixel_width: 0,
            pixel_height: 0,
        })?;
        Ok(())
    }
}

impl Drop for PtyProcess {
    fn drop(&mut self) {
        // Kill child process on drop. Use unwrap_or_else to handle poisoned mutex
        // (panicking in Drop during unwinding would abort the process).
        let mut child = self.child.lock().unwrap_or_else(|e| e.into_inner());
        let _ = child.kill();
        let _ = child.wait();
    }
}

/// Reader for PTY output
pub struct PtyReader {
    reader: Arc<Mutex<Box<dyn Read + Send>>>,
}

impl PtyReader {
    /// Read from PTY (blocking)
    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        let mut reader = self.reader.lock().unwrap_or_else(|e| e.into_inner());
        Ok(reader.read(buf)?)
    }
}

/// Get the default shell for the current platform
fn get_shell() -> String {
    if cfg!(windows) {
        // Try PowerShell first, then cmd
        if std::process::Command::new("pwsh")
            .arg("--version")
            .output()
            .is_ok()
        {
            "pwsh".to_string()
        } else {
            "powershell".to_string()
        }
    } else {
        std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string())
    }
}

/// Get shell arguments
fn get_shell_args(shell: &str) -> Vec<String> {
    if cfg!(windows) {
        if shell.contains("pwsh") || shell.contains("powershell") {
            vec!["-NoLogo".to_string()]
        } else {
            vec![]
        }
    } else {
        vec![]
    }
}

/// Get current working directory
fn get_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| {
            if cfg!(windows) {
                "C:\\".to_string()
            } else {
                "/".to_string()
            }
        })
}

// Agent capture is ephemeral and separate from the user's Win+PrintScreen action.
use super::AgentError;

#[cfg(windows)]
const SCRIPT: &str = r#"
# without DPI awareness a scaled display reports a smaller virtual screen and only its corner is copied
Add-Type -Namespace JarvisCapture -Name Dpi -MemberDefinition '[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();'
[void][JarvisCapture.Dpi]::SetProcessDPIAware()
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bitmap = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bitmap)
try {
  $g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bitmap.Size)
  $max = 1600
  $scale = [Math]::Min(1.0, $max / [double][Math]::Max($b.Width, $b.Height))
  $small = New-Object System.Drawing.Bitmap ([int]($b.Width * $scale)), ([int]($b.Height * $scale))
  try {
    $sg = [System.Drawing.Graphics]::FromImage($small)
    try { $sg.DrawImage($bitmap, 0, 0, $small.Width, $small.Height) } finally { $sg.Dispose() }
    $small.Save($env:JARVIS_CAPTURE, [System.Drawing.Imaging.ImageFormat]::Png)
  } finally { $small.Dispose() }
} finally { $g.Dispose(); $bitmap.Dispose() }
"#;

pub fn capture() -> Result<String, AgentError> {
    #[cfg(windows)]
    {
        // a directory, not an open temp file: GDI+ cannot save over a file another handle holds
        let dir = tempfile::Builder::new()
            .prefix("jarvis-screen-")
            .tempdir()
            .map_err(|e| { warn!("Screen capture: {}", e); AgentError::ToolError })?;
        let target = dir.path().join("screen.png");

        crate::actions::platform::powershell(SCRIPT, &[("JARVIS_CAPTURE", &target.to_string_lossy())])
            .map_err(|e| { warn!("Screen capture failed: {}", e); AgentError::ToolError })?;
        let bytes = std::fs::read(&target).map_err(|e| { warn!("Screen capture: {}", e); AgentError::ToolError })?;
        image_url(&bytes)
    }
    #[cfg(not(windows))]
    {
        Err(AgentError::ToolError)
    }
}

pub fn image_url(bytes: &[u8]) -> Result<String, AgentError> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() > 5 * 1024 * 1024 {
        return Err(AgentError::InvalidResponse);
    }
    // Encode locally without introducing a dependency into the voice core.
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::from("data:image/png;base64,");
    for chunk in bytes.chunks(3) {
        let a = chunk[0] as usize;
        let b = chunk.get(1).copied().unwrap_or(0) as usize;
        let c = chunk.get(2).copied().unwrap_or(0) as usize;
        out.push(TABLE[a >> 2] as char);
        out.push(TABLE[((a & 3) << 4) | (b >> 4)] as char);
        out.push(if chunk.len() > 1 {
            TABLE[((b & 15) << 2) | (c >> 6)] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            TABLE[c & 63] as char
        } else {
            '='
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn capture_script_parses_in_windows_powershell() {
        crate::actions::platform::powershell(
            "[void][scriptblock]::Create($env:JARVIS_SCRIPT)",
            &[("JARVIS_SCRIPT", SCRIPT)],
        )
        .unwrap();
    }
    // the real capture on the CI desktop: the old one saved over an open temp file and always failed
    #[cfg(windows)]
    #[test]
    fn the_screen_is_captured_as_png() {
        let url = capture().unwrap();
        assert!(url.starts_with("data:image/png;base64,iVBORw0KGgo"), "{}", &url[..40.min(url.len())]);
    }
    #[test]
    fn only_bounded_png_data_is_attached() {
        assert_eq!(
            image_url(b"\x89PNG\r\n\x1a\n").unwrap(),
            "data:image/png;base64,iVBORw0KGgo="
        );
        assert!(image_url(b"secret text").is_err());
        let mut data = vec![0; 5 * 1024 * 1024 + 1];
        data[..8].copy_from_slice(b"\x89PNG\r\n\x1a\n");
        assert!(image_url(&data).is_err());
    }
}

// Agent capture is ephemeral and separate from the user's Win+PrintScreen action.
use super::AgentError;

#[cfg(windows)]
const SCRIPT: &str = r#"
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
        let tmp = tempfile::Builder::new()
            .prefix("jarvis-screen-")
            .suffix(".png")
            .tempfile()
            .map_err(|_| AgentError::ToolError)?;
        let target = tmp.path().to_string_lossy().to_string();

        crate::actions::platform::powershell(SCRIPT, &[("JARVIS_CAPTURE", &target)])
            .map_err(|_| AgentError::ToolError)?;
        let bytes = std::fs::read(tmp.path()).map_err(|_| AgentError::ToolError)?;
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

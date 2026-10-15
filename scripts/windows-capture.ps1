# Capture the real Sfoglia window on Windows from a non-interactive shell.
#
#   pwsh scripts/windows-capture.ps1 -Out .zen/evidence/runs/C-006-light.png -Args tests/fixtures/sample.log -Keep
#   pwsh scripts/windows-capture.ps1 -Out .zen/evidence/runs/C-006-dark.png -ProcessId 1234
#
# Starts target/debug/sfoglia.exe with -Args (or captures the window of an already running
# -ProcessId), waits -Wait seconds for the first frame, and saves a PNG through
# PrintWindow(hwnd, PW_RENDERFULLCONTENT). CopyFromScreen gives a black frame from the agent
# shell, and the shell cannot bring a window to the foreground, so key chords in the running
# window are pressed by a person; -Keep leaves the app running for that and prints its pid.
param(
    [Parameter(Mandatory = $true)] [string] $Out,
    [string[]] $Args = @(),
    [int] $ProcessId = 0,
    [int] $Wait = 4,
    [switch] $Keep
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class SfogliaCapture {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out RECT rect);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
$started = $false
if ($ProcessId -gt 0) {
    $process = Get-Process -Id $ProcessId
} else {
    $exe = Join-Path $PSScriptRoot '..\target\debug\sfoglia.exe'
    $process = Start-Process -FilePath $exe -ArgumentList $Args -PassThru
    $started = $true
    Start-Sleep -Seconds $Wait
}
$process.Refresh()
$hwnd = $process.MainWindowHandle
if ($hwnd -eq [IntPtr]::Zero) { throw "process $($process.Id) has no main window" }
$rect = New-Object SfogliaCapture+RECT
[SfogliaCapture]::GetWindowRect($hwnd, [ref] $rect) | Out-Null
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
$ok = [SfogliaCapture]::PrintWindow($hwnd, $hdc, 2)
$graphics.ReleaseHdc($hdc)
$graphics.Dispose()
$full = [System.IO.Path]::GetFullPath((Join-Path (Get-Location) $Out))
$bitmap.Save($full, [System.Drawing.Imaging.ImageFormat]::Png)
$centre = $bitmap.GetPixel([int]($width / 2), [int]($height / 2))
$bitmap.Dispose()
Write-Output ("pid={0} title='{1}' hwnd={2} size={3}x{4} printed={5} centre=#{6:X2}{7:X2}{8:X2} png={9}" -f `
    $process.Id, $process.MainWindowTitle, $hwnd, $width, $height, $ok, $centre.R, $centre.G, $centre.B, $full)
if ($started -and -not $Keep) { Stop-Process -Id $process.Id -Force }

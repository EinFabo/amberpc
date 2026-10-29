# Saves a screenshot for checking the app by eye: the window titled "Amber" (the settings),
# with -Screen the whole primary screen, with -Region x,y,w,h a part of it; at most 1600 px wide.
param([string]$Out = "$PSScriptRoot\..\shot.png", [switch]$Screen, [int[]]$Region)

Add-Type -AssemblyName System.Drawing
Add-Type -AssemblyName System.Windows.Forms
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class AmberUi {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern IntPtr FindWindow(string cls, string title);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
  // The largest visible top-level window of a process: the settings, not the bubble.
  public static IntPtr Largest(uint pid) {
    IntPtr best = IntPtr.Zero; long area = 0;
    EnumWindows((h, p) => {
      uint owner; GetWindowThreadProcessId(h, out owner);
      if (owner != pid || !IsWindowVisible(h)) return true;
      RECT r; GetWindowRect(h, out r);
      long a = (long)(r.Right - r.Left) * (r.Bottom - r.Top);
      if (a > area) { area = a; best = h; }
      return true;
    }, IntPtr.Zero);
    return best;
  }
}
'@
[void][AmberUi]::SetProcessDPIAware()

if ($Region) {
  $x = $Region[0]; $y = $Region[1]; $w = $Region[2]; $h = $Region[3]
} elseif ($Screen) {
  $b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds
  $x = $b.Left; $y = $b.Top; $w = $b.Width; $h = $b.Height
} else {
  $proc = Get-Process -Name amber -ErrorAction SilentlyContinue | Select-Object -First 1
  if (-not $proc) { Write-Output "Amber is not running"; exit 1 }
  $hwnd = [AmberUi]::Largest([uint32]$proc.Id)
  if ($hwnd -eq [IntPtr]::Zero) { Write-Output "Amber settings window not found"; exit 1 }
  [void][AmberUi]::ShowWindow($hwnd, 9)
  [void][AmberUi]::SetForegroundWindow($hwnd)
  Start-Sleep -Milliseconds 600
  $r = New-Object AmberUi+RECT
  [void][AmberUi]::GetWindowRect($hwnd, [ref]$r)
  $x = $r.Left; $y = $r.Top; $w = $r.Right - $r.Left; $h = $r.Bottom - $r.Top
}
$bmp = New-Object System.Drawing.Bitmap $w, $h
$gfx = [System.Drawing.Graphics]::FromImage($bmp)
$gfx.CopyFromScreen($x, $y, 0, 0, $bmp.Size)
$scale = [Math]::Min(1.0, 1600 / $w)
$small = New-Object System.Drawing.Bitmap ([int]($w * $scale)), ([int]($h * $scale))
$g2 = [System.Drawing.Graphics]::FromImage($small)
$g2.InterpolationMode = [System.Drawing.Drawing2D.InterpolationMode]::HighQualityBicubic
$g2.DrawImage($bmp, 0, 0, $small.Width, $small.Height)
$small.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Output "saved: $Out ($w x $h)"

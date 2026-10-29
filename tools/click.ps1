# Clicks at (X, Y) inside Amber's settings window, for checking pages by eye.
param([int]$X = 0, [int]$Y = 0)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class AmberClick {
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
  [DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
  public delegate bool EnumProc(IntPtr h, IntPtr p);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr p);
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int Left, Top, Right, Bottom; }
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
[void][AmberClick]::SetProcessDPIAware()
$proc = Get-Process -Name amber -ErrorAction SilentlyContinue | Select-Object -First 1
if (-not $proc) { Write-Output "Amber is not running"; exit 1 }
$hwnd = [AmberClick]::Largest([uint32]$proc.Id)
[void][AmberClick]::SetForegroundWindow($hwnd)
Start-Sleep -Milliseconds 300
$r = New-Object AmberClick+RECT
[void][AmberClick]::GetWindowRect($hwnd, [ref]$r)
[void][AmberClick]::SetCursorPos(($r.Left + $X), ($r.Top + $Y))
Start-Sleep -Milliseconds 100
[AmberClick]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)
Start-Sleep -Milliseconds 50
[AmberClick]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)

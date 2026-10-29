# Presses Ctrl+Alt+Space (or -Keys) once via SendInput, for testing the hotkey path.
param([int]$HoldMs = 60)
Add-Type @'
using System;
using System.Runtime.InteropServices;
public class AmberKeys {
  [DllImport("user32.dll")] public static extern void keybd_event(byte vk, byte scan, uint flags, UIntPtr extra);
}
'@
$keys = @(0x11, 0x12, 0x20) # Ctrl, Alt, Space
foreach ($k in $keys) { [AmberKeys]::keybd_event($k, 0, 0, [UIntPtr]::Zero) }
Start-Sleep -Milliseconds $HoldMs
[array]::Reverse($keys)
foreach ($k in $keys) { [AmberKeys]::keybd_event($k, 0, 2, [UIntPtr]::Zero) }

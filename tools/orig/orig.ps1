# Drive the running original (NFS13.exe) on Windows: focus, key input (scancodes), window screenshot.
#   orig.ps1 focus | info | shot <png> | steps "hold A 3; tap Return; wait 1; hold A+Left 2"
# Keys are only sent while the game's window is the foreground window; otherwise the step aborts.
param([Parameter(Mandatory = $true)][string]$Cmd, [string]$Arg = "")

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Runtime.InteropServices;
using System.Threading;
public static class Orig {
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int L, T, R, B; }
    [StructLayout(LayoutKind.Sequential)] struct POINT { public int X, Y; }
    [StructLayout(LayoutKind.Sequential)] struct KEYBDINPUT { public ushort wVk, wScan; public uint dwFlags, time; public IntPtr extra; }
    [StructLayout(LayoutKind.Explicit)] struct INPUT {
        [FieldOffset(0)] public uint type;
        [FieldOffset(8)] public KEYBDINPUT ki;
        [FieldOffset(8)] long pad0; [FieldOffset(16)] long pad1; [FieldOffset(24)] long pad2; [FieldOffset(32)] long pad3;
    }
    [DllImport("user32.dll")] static extern uint SendInput(uint n, INPUT[] inputs, int size);
    [DllImport("user32.dll")] static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] static extern bool IsIconic(IntPtr h);
    [DllImport("user32.dll")] static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern bool ClientToScreen(IntPtr h, ref POINT p);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool AttachThreadInput(uint a, uint b, bool attach);
    [DllImport("kernel32.dll")] static extern uint GetCurrentThreadId();
    [DllImport("user32.dll")] static extern bool SetProcessDPIAware();

    public static bool Foreground(IntPtr h) { return GetForegroundWindow() == h; }

    public static bool Focus(IntPtr h) {
        if (IsIconic(h)) ShowWindow(h, 9);
        if (Foreground(h)) return true;
        uint pid; uint fg = GetWindowThreadProcessId(GetForegroundWindow(), out pid); uint me = GetCurrentThreadId();
        Key(0x38, false, true); Key(0x38, false, false);   // an Alt tap lifts the foreground lock
        AttachThreadInput(me, fg, true);
        SetForegroundWindow(h);
        AttachThreadInput(me, fg, false);
        Thread.Sleep(400);
        return Foreground(h);
    }

    public static void Key(ushort scan, bool extended, bool down) {
        var i = new INPUT[1];
        i[0].type = 1;
        i[0].ki.wScan = scan;
        i[0].ki.dwFlags = 0x0008u | (extended ? 0x0001u : 0u) | (down ? 0u : 0x0002u);
        SendInput(1, i, Marshal.SizeOf(typeof(INPUT)));
    }

    public static string Rect(IntPtr h) {
        SetProcessDPIAware();
        RECT r; GetClientRect(h, out r); var p = new POINT(); ClientToScreen(h, ref p);
        return p.X + "," + p.Y + " " + (r.R - r.L) + "x" + (r.B - r.T);
    }

    // The game's client area only, and only while it is in front (otherwise other windows would be in it).
    public static bool Shot(IntPtr h, string path, int maxWidth) {
        SetProcessDPIAware();
        if (!Foreground(h)) return false;
        RECT r; GetClientRect(h, out r); var p = new POINT(); ClientToScreen(h, ref p);
        int w = r.R - r.L, ht = r.B - r.T; if (w <= 0 || ht <= 0) return false;
        using (var bmp = new Bitmap(w, ht)) {
            using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(p.X, p.Y, 0, 0, new Size(w, ht));
            if (w > maxWidth) { using (var s = new Bitmap(bmp, maxWidth, ht * maxWidth / w)) s.Save(path); } else bmp.Save(path);
        }
        return true;
    }
}
'@

$proc = Get-Process -Name NFS13 -ErrorAction SilentlyContinue | Sort-Object WorkingSet64 -Descending | Select-Object -First 1
if (-not $proc) { Write-Output "NFS13.exe is not running"; exit 1 }
$h = $proc.MainWindowHandle
if ($h -eq [IntPtr]::Zero) { Write-Output "the game has no window"; exit 1 }

$keys = @{
    'A' = @(0x1E, $false); 'Z' = @(0x2C, $false); 'Space' = @(0x39, $false); 'Return' = @(0x1C, $false); 'Esc' = @(0x01, $false)
    'Left' = @(0x4B, $true); 'Right' = @(0x4D, $true); 'Up' = @(0x48, $true); 'Down' = @(0x50, $true)
    'LShift' = @(0x2A, $false); 'W' = @(0x11, $false); 'S' = @(0x1F, $false); 'D' = @(0x20, $false); 'C' = @(0x2E, $false)
    'Tab' = @(0x0F, $false); 'E' = @(0x12, $false); 'X' = @(0x2D, $false)
}

function Focus-Game {
    if ([Orig]::Focus($h)) { return $true }
    for ($i = 0; $i -lt 3; $i++) {
        [void](New-Object -ComObject WScript.Shell).AppActivate($proc.Id)
        Start-Sleep -Milliseconds 600
        if ([Orig]::Foreground($h)) { return $true }
    }
    return [Orig]::Focus($h)
}

function Send-Keys([string[]]$names, [bool]$down) {
    foreach ($n in $names) { $k = $keys[$n]; if (-not $k) { throw "unknown key $n" }; [Orig]::Key([uint16]$k[0], [bool]$k[1], $down) }
}

switch ($Cmd) {
    'info' { "pid $($proc.Id) hwnd $h title '$($proc.MainWindowTitle)' foreground $([Orig]::Foreground($h)) client $([Orig]::Rect($h))" }
    'focus' { "focused: $(Focus-Game)" }
    'shot' { if (-not (Focus-Game)) { "could not focus the game"; exit 1 }; Start-Sleep -Milliseconds 300; "shot: $([Orig]::Shot($h, $Arg, 1280))" }
    'steps' {
        if (-not (Focus-Game)) { "could not focus the game"; exit 1 }
        foreach ($step in ($Arg -split ';')) {
            $w = ($step.Trim() -split '\s+'); if (-not $w[0]) { continue }
            switch ($w[0]) {
                'wait' { Start-Sleep -Milliseconds ([int]([double]$w[1] * 1000)) }
                'shot' { "shot $($w[1]): $([Orig]::Shot($h, $w[1], 1280))" }
                { $_ -in 'hold', 'tap' } {
                    $names = $w[1] -split '\+'
                    $ms = if ($w[0] -eq 'tap') { 80 } else { [int]([double]$w[2] * 1000) }
                    if (-not [Orig]::Foreground($h)) { "lost focus before '$step': stopping"; exit 2 }
                    Send-Keys $names $true
                    try {
                        $end = (Get-Date).AddMilliseconds($ms)
                        while ((Get-Date) -lt $end) { if (-not [Orig]::Foreground($h)) { "lost focus during '$step': stopping"; Send-Keys $names $false; exit 2 }; Start-Sleep -Milliseconds 20 }
                    } finally { Send-Keys $names $false }
                    Start-Sleep -Milliseconds 60
                }
                default { throw "unknown step '$step'" }
            }
        }
        "steps done"
    }
    default { "unknown command $Cmd"; exit 1 }
}

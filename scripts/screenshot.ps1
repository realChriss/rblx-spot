$ErrorActionPreference = 'Stop'
$root = Split-Path $PSScriptRoot
Set-Location $root

$src = Get-Content src/popup.rs -Raw
$w = [double]([regex]::Match($src, 'const W: f32 = ([\d.]+);').Groups[1].Value)
$h = [double]([regex]::Match($src, 'const H: f32 = ([\d.]+);').Groups[1].Value)

cargo build --release
if ($LASTEXITCODE) { exit $LASTEXITCODE }
if (Get-Process rblx-spot -ErrorAction SilentlyContinue) { throw 'Close the running rblx-spot first.' }

Add-Type -ReferencedAssemblies System.Drawing, System.Windows.Forms -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.Runtime.InteropServices;
using System.Windows.Forms;
public static class Shot {
    [StructLayout(LayoutKind.Sequential)] struct RECT { public int l, t, r, b; }
    [DllImport("user32.dll", CharSet = CharSet.Unicode)] static extern IntPtr FindWindowW(string cls, string title);
    [DllImport("user32.dll")] static extern IntPtr SetThreadDpiAwarenessContext(IntPtr ctx);
    [DllImport("user32.dll")] static extern uint GetDpiForWindow(IntPtr h);
    [DllImport("user32.dll")] static extern bool SetWindowPos(IntPtr h, IntPtr after, int x, int y, int w, int hgt, uint flags);
    [DllImport("user32.dll")] static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
    [DllImport("user32.dll")] static extern bool PostThreadMessageW(uint tid, uint msg, IntPtr wp, IntPtr lp);
    public static IntPtr Find() { return FindWindowW("rblx-spot-popup", null); }

    static Bitmap Grab(Form backdrop, Color c, Rectangle r) {
        backdrop.BackColor = c;
        backdrop.Refresh();
        Application.DoEvents();
        System.Threading.Thread.Sleep(600);
        var bmp = new Bitmap(r.Width, r.Height, PixelFormat.Format32bppArgb);
        using (var g = Graphics.FromImage(bmp)) g.CopyFromScreen(r.Location, Point.Empty, r.Size);
        return bmp;
    }

    public static void Capture(IntPtr h, double dw, double dh, string path) {
        SetThreadDpiAwarenessContext(new IntPtr(-4));
        double s = GetDpiForWindow(h) / 96.0;
        int w = (int)Math.Round(dw * s), hgt = (int)Math.Round(dh * s);
        var backdrop = new Form { FormBorderStyle = FormBorderStyle.None, StartPosition = FormStartPosition.Manual,
            ShowInTaskbar = false, TopMost = true, Bounds = new Rectangle(0, 0, w + 400, hgt + 400) };
        backdrop.Show();
        SetWindowPos(h, new IntPtr(-1), 200, 200, w, hgt, 0x0040 | 0x0010);
        RECT wr; GetWindowRect(h, out wr);
        var r = Rectangle.FromLTRB(wr.l, wr.t, wr.r, wr.b);
        using (var black = Grab(backdrop, Color.Black, r))
        using (var white = Grab(backdrop, Color.White, r))
        using (var outp = new Bitmap(r.Width, r.Height, PixelFormat.Format32bppArgb)) {
            for (int y = 0; y < r.Height; y++)
                for (int x = 0; x < r.Width; x++) {
                    Color b = black.GetPixel(x, y), wh = white.GetPixel(x, y);
                    int a = Math.Max(0, Math.Min(255, 255 - (wh.G - b.G)));
                    Func<int, int> un = v => a == 0 ? 0 : Math.Min(255, v * 255 / a);
                    outp.SetPixel(x, y, Color.FromArgb(a, un(b.R), un(b.G), un(b.B)));
                }
            outp.Save(path, ImageFormat.Png);
        }
        backdrop.Close();
    }

    public static void Quit(IntPtr h) {
        uint pid;
        PostThreadMessageW(GetWindowThreadProcessId(h, out pid), 0x0012, IntPtr.Zero, IntPtr.Zero);
    }
}
'@

$appdata = Join-Path ([IO.Path]::GetTempPath()) "rblx-spot-shot-$PID"
$oldAppdata = $env:APPDATA
$env:APPDATA = $appdata
$p = Start-Process target/release/rblx-spot.exe -PassThru
$env:APPDATA = $oldAppdata
try {
    $hwnd = [IntPtr]::Zero
    for ($i = 0; $i -lt 100 -and $hwnd -eq [IntPtr]::Zero; $i++) { Start-Sleep -Milliseconds 100; $hwnd = [Shot]::Find() }
    if ($hwnd -eq [IntPtr]::Zero) { throw 'popup window not found' }
    Start-Sleep -Seconds 3
    [Shot]::Capture($hwnd, $w, $h, (Join-Path $root 'popup.png'))
    [Shot]::Quit($hwnd)
    if (-not $p.WaitForExit(3000)) { Stop-Process $p }
} catch {
    Stop-Process $p -ErrorAction SilentlyContinue
    throw
} finally {
    Remove-Item $appdata -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host 'Wrote popup.png'

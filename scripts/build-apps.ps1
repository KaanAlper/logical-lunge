# ii AppSearch karşılığı: Windows'un uygulama listesini (shell:AppsFolder — Başlat menüsünün
# kullandığı liste) yerelleştirilmiş adları ("Ekran Klavyesi" gibi) ve gerçek ikonlarıyla
# apps.json'a yazar. Belgeler/yardım/kaldırma kısayolları elenir.
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;

public static class ShellIcon
{
    [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    interface IShellItemImageFactory { void GetImage(SIZE size, int flags, out IntPtr phbm); }
    [StructLayout(LayoutKind.Sequential)] struct SIZE { public int cx, cy; }
    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, [MarshalAs(UnmanagedType.LPStruct)] Guid riid, [MarshalAs(UnmanagedType.Interface)] out IShellItemImageFactory item);
    [DllImport("gdi32.dll")] static extern bool DeleteObject(IntPtr o);

    public static string Png(string parsingName, int size)
    {
        try
        {
            IShellItemImageFactory f;
            SHCreateItemFromParsingName(parsingName, IntPtr.Zero, typeof(IShellItemImageFactory).GUID, out f);
            IntPtr hbm;
            f.GetImage(new SIZE { cx = size, cy = size }, 0x1 /*BIGGERSIZEOK*/ | 0x4 /*ICONONLY*/, out hbm);
            try
            {
                // Alfa kanalını koruyarak kopyala (Image.FromHbitmap alfayı atar)
                var src = Image.FromHbitmap(hbm);
                var bmp = new Bitmap(src.Width, src.Height, PixelFormat.Format32bppArgb);
                var data = src.LockBits(new Rectangle(0, 0, src.Width, src.Height), ImageLockMode.ReadOnly, src.PixelFormat);
                var dst = new Bitmap(src.Width, src.Height, data.Stride, PixelFormat.Format32bppArgb, data.Scan0);
                using (var g = Graphics.FromImage(bmp)) g.DrawImage(dst, 0, 0);
                src.UnlockBits(data);
                using (var ms = new MemoryStream()) { bmp.Save(ms, ImageFormat.Png); return "data:image/png;base64," + Convert.ToBase64String(ms.ToArray()); }
            }
            finally { DeleteObject(hbm); }
        }
        catch { return null; }
    }
}
'@

$out = Join-Path $PSScriptRoot '..\ui\logical-lunge\apps.json'
$skip = '(?i)(uninstall|kaldır|readme|beni oku|help|yardım|documentation|belgeler|release notes|license|lisans|website|web sitesi|manual|kılavuz|changelog|what''s new)'

$shell = New-Object -ComObject Shell.Application
$items = $shell.NameSpace('shell:AppsFolder').Items()
$apps = New-Object System.Collections.Generic.List[object]
$seen = @{}
foreach ($it in $items) {
    $name = $it.Name
    $id = $it.Path
    if (-not $name -or $name -match $skip) { continue }
    if ($id -match '^https?:' -or $id -match '\.(txt|pdf|html?|chm|url|md|rtf)$') { continue }
    $key = $name.ToLower()
    if ($seen.ContainsKey($key)) { continue }
    $seen[$key] = $true
    $exe = $null
    if ($id -match '\\([^\\]+)\.exe$') { $exe = $Matches[1].ToLower() }
    # Win+R penceresi: 'run' yazınca da bulunsun
    $alias = $null
    if ($id -eq 'Microsoft.Windows.Shell.RunDialog') { $name = "$name (Run)"; $alias = 'run' }
    $apps.Add([ordered]@{
        name = $name
        path = "shell:AppsFolder\$id"
        exe  = $exe
        alias = $alias
        icon = [ShellIcon]::Png("shell:AppsFolder\$id", 48)
    })
}

$sorted = $apps | Sort-Object { $_.name }
[IO.File]::WriteAllText($out, (ConvertTo-Json @($sorted) -Depth 3 -Compress), (New-Object Text.UTF8Encoding $false))
"$($apps.Count) uygulama -> $out"

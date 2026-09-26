# ii AppSearch karşılığı: Windows'un uygulama listesini (shell:AppsFolder — Başlat menüsünün
# kullandığı liste) yerelleştirilmiş adları ("Ekran Klavyesi" gibi) ve gerçek ikonlarıyla
# apps.json'a yazar. Belgeler/yardım/kaldırma kısayolları elenir.
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @'
using System;
using System.Drawing;
using System.Drawing.Imaging;
using System.IO;
using System.Runtime.InteropServices;
using System.Text;

public static class ShellIcon
{
    [DllImport("shlwapi.dll", CharSet = CharSet.Unicode)] static extern int SHLoadIndirectString(string src, StringBuilder buf, int cch, IntPtr reserved);
    // "@%SystemRoot%\system32\Taskmgr.exe,-32420" -> "Görev Yöneticisi" (kullanıcının arayüz dilinde)
    public static string Indirect(string s)
    {
        var sb = new StringBuilder(512);
        return SHLoadIndirectString(Environment.ExpandEnvironmentVariables(s), sb, sb.Capacity, IntPtr.Zero) == 0 && sb.Length > 0 ? sb.ToString() : null;
    }

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

# Kurulum klasörü yönetici korumalı: liste kullanıcının veri klasöründe; widget'lar çekirdekten okur (/apps.json)
$out = Join-Path $env:LOCALAPPDATA 'LogicalLunge\state\apps.json'
New-Item -ItemType Directory -Force (Split-Path $out) | Out-Null
$skip = '(?i)(uninstall|kaldır|readme|beni oku|help|yardım|documentation|belgeler|release notes|license|lisans|website|web sitesi|manual|kılavuz|changelog|what''s new)'

# Başlat menüsü kısayollarının yerelleştirilmiş adları (klasörlerindeki desktop.ini [LocalizedFileNames]). AppsFolder bazı
# sistem kısayollarını dosya adıyla veriyor ("Task Manager"), Başlat menüsü ise bu kaynaktan çözüp Türkçe gösteriyor.
$localized = @{}
foreach ($root in @("$env:ProgramData\Microsoft\Windows\Start Menu\Programs", "$env:APPDATA\Microsoft\Windows\Start Menu\Programs")) {
    foreach ($ini in (Get-ChildItem -LiteralPath $root -Recurse -Force -Filter desktop.ini -ErrorAction SilentlyContinue)) {
        $inFiles = $false
        try { $lines = [IO.File]::ReadAllLines($ini.FullName) } catch { continue }
        foreach ($line in $lines) {
            if ($line -match '^\s*\[(.+)\]') { $inFiles = $Matches[1] -eq 'LocalizedFileNames'; continue }
            if ($inFiles -and $line -match '^(.+?)\.lnk=(.+)$') {
                $val = $Matches[2].Trim()
                $nm = if ($val.StartsWith('@')) { [ShellIcon]::Indirect($val) } else { $val }
                if ($nm) { $localized[$Matches[1].Trim().ToLower()] = $nm }
            }
        }
    }
}

$shell = New-Object -ComObject Shell.Application
$items = $shell.NameSpace('shell:AppsFolder').Items()
$apps = New-Object System.Collections.Generic.List[object]
$seen = @{}
foreach ($it in $items) {
    $name = $it.Name
    $id = $it.Path
    $also = $null
    if ($name -and $localized.ContainsKey($name.ToLower())) { $also = $name; $name = $localized[$name.ToLower()] } # İngilizce adla da bulunur
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
        also = $also
        icon = [ShellIcon]::Png("shell:AppsFolder\$id", 48)
    })
}

$sorted = $apps | Sort-Object { $_.name }
[IO.File]::WriteAllText($out, (ConvertTo-Json @($sorted) -Depth 3 -Compress), (New-Object Text.UTF8Encoding $false))
"$($apps.Count) uygulama -> $out"

# ii scripts/colors/terminal/kitty-theme.conf şablonunun WezTerm karşılığı.
# ii'nin kendi üreticisini (generate_colors_material.py, materialyoucolor==2.0.10) duvar kağıdıyla
# ii'nin varsayılanlarıyla çalıştırır ve ~/.config/wezterm/ll-colors.lua yazar. WezTerm bu dosyayı
# izlediği için duvar kağıdı değişip betik yeniden çalışınca açık terminaller de anında güncellenir.
#   python wezterm-colors.py [--path duvar.jpg] [--mode dark|light]
import argparse, os, subprocess, sys, winreg

HERE = os.path.dirname(os.path.abspath(__file__))
ap = argparse.ArgumentParser()
ap.add_argument("--path")
ap.add_argument("--mode", default="dark")
args = ap.parse_args()

path = args.path
if not path:
    with winreg.OpenKey(winreg.HKEY_CURRENT_USER, r"Control Panel\Desktop") as k:
        path = winreg.QueryValueEx(k, "WallPaper")[0]
if not path or not os.path.exists(path):
    path = os.path.join(os.environ["APPDATA"], r"Microsoft\Windows\Themes\TranscodedWallpaper")

# ii switchwall.sh + Config.qml varsayılanları: harmony 0.6, harmonizeThreshold 100, termFgBoost 0.35, --blend_bg_fg
# ii'nin betiğini aynı süreçte çalıştır (tek exe olarak paketlenince ayrı python yok)
import io, runpy, contextlib, warnings
warnings.filterwarnings("ignore")
base = getattr(sys, "_MEIPASS", HERE)
argv0 = sys.argv
sys.argv = ["generate_colors_material.py",
            "--path", path, "--mode", args.mode, "--scheme", "scheme-tonal-spot",
            "--termscheme", os.path.join(base, "scheme-base.json"), "--blend_bg_fg",
            "--harmony", "0.6", "--harmonize_threshold", "100", "--term_fg_boost", "0.35"]
buf = io.StringIO()
with contextlib.redirect_stdout(buf):
    runpy.run_path(os.path.join(base, "generate_colors_material.py"), run_name="__main__")
sys.argv = argv0
out = buf.getvalue()

c = {}
for line in out.splitlines():
    if line.startswith("$") and ":" in line:
        k, v = line[1:].split(":", 1)
        c[k.strip()] = v.strip().rstrip(";")

t = lambda i: c[f"term{i}"]
indexed = {  # kitty-theme.conf: starship istemi için "gri" indeksler Material renkleriyle ezilir
    255: c["primary"], 254: c["primaryContainer"], 253: c["secondary"], 252: c["secondaryContainer"],
    251: c["tertiary"], 250: c["tertiaryContainer"], 249: c["error"], 248: c["errorContainer"],
    232: c["onPrimary"], 233: c["onPrimaryContainer"], 234: c["onSecondary"], 235: c["onSecondaryContainer"],
    236: c["onTertiary"], 237: c["onTertiaryContainer"], 238: c["onError"], 239: c["onErrorContainer"],
    240: c["onPrimary"], 243: c["primary"], 244: c["error"], 245: c["outlineVariant"],
}
q = lambda s: f"'{s}'"
lua = ["-- tools\\termcolors\\wezterm-colors.py tarafından üretildi (ii kitty-theme.conf eşlemesi). Elle düzenleme.",
       "return {",
       f"  background = {q(t(0))}, foreground = {q(t(7))},",
       f"  cursor_bg = {q(t(7))}, cursor_border = {q(t(7))}, cursor_fg = {q(t(0))},",
       f"  selection_bg = {q(c['onSecondaryContainer'])}, selection_fg = {q(c['secondaryContainer'])},",
       f"  ansi = {{ {', '.join(q(t(i)) for i in range(8))} }},",
       f"  brights = {{ {', '.join(q(t(i)) for i in range(8, 16))} }},",
       "  indexed = { " + ", ".join(f"[{k}] = {q(v)}" for k, v in sorted(indexed.items())) + " },",
       f"  split = {q(c['outlineVariant'])}, scrollbar_thumb = {q(c['outlineVariant'])},",
       "  tab_bar = {",
       f"    background = {q(t(0))},",
       f"    active_tab = {{ bg_color = {q(c['primaryContainer'])}, fg_color = {q(c['onPrimaryContainer'])} }},",
       f"    inactive_tab = {{ bg_color = {q(t(0))}, fg_color = {q(t(7))} }},",
       f"    inactive_tab_hover = {{ bg_color = {q(c['secondaryContainer'])}, fg_color = {q(c['onSecondaryContainer'])} }},",
       f"    new_tab = {{ bg_color = {q(t(0))}, fg_color = {q(t(7))} }},",
       f"    new_tab_hover = {{ bg_color = {q(c['secondaryContainer'])}, fg_color = {q(c['onSecondaryContainer'])} }},",
       "  },",
       "}", ""]
dst = os.path.join(os.path.expanduser("~"), ".config", "wezterm")
os.makedirs(dst, exist_ok=True)
with open(os.path.join(dst, "ll-colors.lua"), "w", encoding="utf-8") as f:
    f.write("\n".join(lua))
if sys.stdout: print("ok", c["primary"], t(0))  # pencere modunda (exe) stdout yok

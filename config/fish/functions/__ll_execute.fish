# Windows yolları tırnaksız yazılınca (powershell -File C:\Users\...\betik.ps1) fish ters eğik çizgiyi kaçış sanıyordu:
# "\U" Unicode kaçışı olup "Invalid token" veriyor, "\n" sessizce satır sonuna dönüyordu. Enter'a basınca komut
# satırındaki tırnak dışındaki Windows yolları (C:\..., \\sunucu\..., .\ ve ..\ ile başlayanlar) tek tırnağa alınır;
# içlerindeki \ ve ' korunur. Tırnaklı metne, eğik çizgili yollara ve fish'e göre kaçışlanmış kelimelere dokunulmaz.

# Bir satırı düzeltip yazar (test edilebilsin diye komut satırından bağımsız)
function __ll_winpath_line
    set -l line $argv[1]
    # Hızlı yol: ters eğik çizgi yoksa olduğu gibi
    if not string match -qr -- '\\\\' $line
        printf '%s' $line
        return
    end
    set -l out ''
    set -l word ''
    set -l keep 0   # kelime tırnak ya da kaçış içeriyor: dokunma
    set -l q ''     # açık tırnak
    set -l esc 0
    for c in (string split -- '' $line)
        if test -n "$q"
            set word "$word$c"
            if test $esc = 1
                set esc 0
            else if test "$q" = '"' -a "$c" = '\\'
                set esc 1
            else if test "$c" = "$q"
                set q ''
            end
            continue
        end
        if test $esc = 1
            # fish kaçışı (ör. "\ " boşluk): kullanıcı bilerek kaçışlamış
            set word "$word$c"
            set esc 0
            continue
        end
        switch $c
            case "'" '"'
                set q $c
                set keep 1
                set word "$word$c"
            case ' ' \t \n ';' '|' '&' '(' ')' '<' '>'
                # ara değişken: boş kelimede "$out"(boş) birleşimi out'u tümden siliyordu (fish kartezyen çarpımı)
                set -l w (__ll_winpath_word "$word" $keep)
                set out "$out$w$c"
                set word ''
                set keep 0
            case '\\'
                set word "$word$c"
                # yol biçimli kelimede \ bir yol ayracıdır; başka kelimede fish kaçışıdır
                if not string match -rq -- '^([A-Za-z]:|\\\\|\.{1,2})' "$word"
                    set esc 1
                    set keep 1
                end
            case '*'
                set word "$word$c"
        end
    end
    set -l w (__ll_winpath_word "$word" $keep)
    printf '%s' "$out$w"
end

function __ll_winpath_word -a word keep
    if test "$keep" = 0; and string match -rq -- '^([A-Za-z]:\\\\|\\\\\\\\|\.{1,2}\\\\)' "$word"
        printf "'%s'" (string replace -a -- '\\' '\\\\' "$word" | string replace -a -- "'" "\\'")
    else
        printf '%s' "$word"
    end
end

function __ll_quote_winpaths
    set -l line (commandline | string collect)
    set -l fixed (__ll_winpath_line "$line" | string collect)
    test "$fixed" = "$line"; or commandline -r -- $fixed
end

# Enter: Windows yollarını tırnağa al, !! / !$'ı aç; fish'in anlamadığı satırı anlayan kabukla çalıştır (aşağıda).
# Eksik satırda her zamanki gibi yeni satır açılır.
function __ll_execute
    __ll_quote_winpaths
    set -l line (commandline | string collect)
    set -l h (__ll_history_line "$line" | string collect)
    if test -n "$h"
        commandline -r -- $h
        set line $h
    end
    if string match -qr -- '\S' "$line"
        commandline --is-valid
        set -l valid $status
        set -l fixed (__ll_foreign_line "$line" $valid | string collect)
        test -n "$fixed"; and commandline -r -- $fixed
    end
    commandline -f execute
end

# Fish'in anlamadığı satırlar (başka kabuktan kopyalanan / alışkanlıkla yazılan): fish'e göre doğruysa fish çalıştırır.
# Değilse satır olduğu gibi, bulunduğun klasörde, anlayan kabukta çalışır (geçmişte de öyle görünür):
#   - PowerShell: $env:X, fish'in bulamadığı Fiil-İsim komutu (Get-ChildItem ...)
#   - bash: fish'e göre hatalı ya da yarım (for ...; do ...; done, if [ ]; then ... fi, $((1+2)), ${HOME}), bash'e göre
#     tam ve geçerli. Yalnızca fish reddedince bash'e sorulur: normal komutlarda ek süreç yok.
#   - "ADI=değer" tek başına: fish'in "set -g ADI değer"i (değişken bu oturumda kalsın)
# Boş çıktı: satıra dokunma.
function __ll_foreign_line -a line valid
    # valid: commandline --is-valid çıkışı (0 doğru, 1 yarım, 2 hatalı)
    set -l first (string match -r -- '^\s*([^\s;|&()<>]+)' $line)[2]
    if string match -qr -- '\$env:[A-Za-z_]' $line
        or begin
            string match -qr -- '^[A-Za-z]+-[A-Za-z][A-Za-z0-9]*$' "$first"
            and not type -q -- "$first"
        end
        printf 'pwsh-run %s' (string escape --style=script -- $line)
        return
    end
    test "$valid" = 0; and return

    set -l m (string match -r -- '^\s*([A-Za-z_][A-Za-z0-9_]*)=(\S*)\s*$' $line)
    if test (count $m) -eq 3
        printf 'set -g %s %s' $m[2] $m[3]
        return
    end

    # yarım: yalnızca bash bloğunun sonu yazılmışsa (fish'in begin / switch / function'ı yarım kalmış olabilir)
    if test "$valid" = 1
        string match -qr -- '(^|[;\s])(then|fi|do|done|esac|elif)($|[;\s])' $line; or return
    end
    set -l bash (command -s bash)
    test -n "$bash"; or return
    if $bash -n -c $line 2>/dev/null
        printf 'bash -c %s' (string escape --style=script -- $line)
    end
end

# !! (son komut) ve !$ (son komutun son kelimesi), bash'teki gibi; tırnak içinde değilken
function __ll_history_line -a line
    string match -qr -- '!(!|\$)' $line; or return 1
    set -l last $history[1]
    test -n "$last"; or return 1
    set -l words (string split -n ' ' -- $last)
    set -l out (string replace -ar -- '(^|\s)!!(\s|$)' "\${1}$last\${2}" $line)
    set out (string replace -ar -- '(^|\s)!\$(\s|$)' "\${1}$words[-1]\${2}" $out)
    test "$out" != "$line"; or return 1
    printf '%s' $out
end

# PowerShell'de çalıştır: varsa PowerShell 7 (hızlı açılır), yoksa Windows PowerShell. MSYS2 satırdaki /yolları Windows
# yoluna çevirmesin.
function pwsh-run
    set -l ps (command -s pwsh; or command -s powershell.exe; or command -s powershell)
    if test -z "$ps[1]"
        echo "PowerShell bulunamadı" >&2
        return 127
    end
    MSYS2_ARG_CONV_EXCL='*' $ps[1] -NoLogo -NoProfile -Command $argv
end


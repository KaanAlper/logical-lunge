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

# Enter: önce yolları düzelt, sonra her zamanki gibi çalıştır (eksik satırda yeni satır açar)
function __ll_execute
    __ll_quote_winpaths
    commandline -f execute
end

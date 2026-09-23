function themecolor --description 'Terminal renk teması: ok tuşlarıyla önizle, Enter ile kalıcı kaydet, Esc ile vazgeç'
    set -l file ~/.config/wezterm/ll-theme
    mkdir -p (path dirname $file)
    set -l current ii
    test -f $file; and set current (string trim -c '"\' ' <$file)

    # "ii" = ii'nin duvar kağıdından ürettiği Material You renkleri; diğerleri WezTerm'in yerleşik temaları
    set -l themes \
        'ii  ·  duvar kağıdından (varsayılan)' \
        'Catppuccin Mocha' 'Catppuccin Macchiato' 'Catppuccin Latte' \
        'Tokyo Night' 'Tokyo Night Storm (Gogh)' 'Rosé Pine (Gogh)' 'Rosé Pine Moon (Gogh)' \
        'Kanagawa (Gogh)' 'Kanagawa Dragon (Gogh)' 'Gruvbox dark, medium (base16)' 'Gruvbox Material (Gogh)' \
        'Everforest Dark Medium (Gogh)' 'Nord (Gogh)' 'Dracula (Official)' 'One Dark (Gogh)' \
        'Material Palenight (base16)' 'Monokai Pro (Gogh)' 'Night Owl (Gogh)' 'Oxocarbon Dark (Gogh)' \
        'Horizon Dark (Gogh)' 'Ayu Mirage' 'GitHub Dark' 'Solarized Dark Higher Contrast' \
        'Gruvbox Light' 'Everforest Light Medium (Gogh)'

    if not command -q fzf
        echo "themecolor: fzf bulunamadı" >&2
        return 1
    end

    # Açıkken şu anki tema seçili gelsin
    set -l pos 1
    for i in (seq (count $themes))
        set -l name (string replace -r '^ii .*' ii -- $themes[$i])
        test "$name" = "$current"; and set pos $i; and break
    end

    # İmleç her temaya geldiğinde dosyaya yaz: WezTerm dosyayı izliyor, önizleme anında görünür
    set -l fish_exe (cygpath -w /usr/bin/fish.exe)
    set -l write "string replace -r '^ii .*' ii -- {} >'$file'"
    set -l sel (printf '%s\n' $themes | fzf --height=55% --layout=reverse --border=rounded --info=hidden \
        --prompt='Tema › ' --pointer='›' --header='↑↓ önizle  ·  Enter kaydet  ·  Esc vazgeç' \
        --color='border:5,prompt:5,pointer:5,header:8' \
        --with-shell="$fish_exe --no-config -c" \
        --bind="load:pos($pos)" --bind="focus:execute-silent($write)")

    if test -z "$sel"
        echo $current >$file
        echo "Tema değişmedi: $current"
        return 0
    end
    set sel (string replace -r '^ii .*' ii -- $sel)
    echo $sel >$file
    echo "Tema: $sel (kalıcı)"
end

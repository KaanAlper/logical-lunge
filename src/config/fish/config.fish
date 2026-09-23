# end-4 dots-hyprland .config/fish/config.fish (Windows / MSYS2 uyarlaması)
if status is-interactive
    # No greeting
    set fish_greeting

    # PATH'i sadeleştir (yalnızca fish içinde; Windows PATH'i değişmez). fish yazarken her tuşta komutun
    # var olup olmadığını TÜM PATH klasörlerinde arıyor; MSYS2 üzerinden her klasör pahalı. 73 girişte
    # arama başına ~13 ms'ydi ve yazarken takılma yapıyordu. Tekrarları, klasör olmayanları (...\agy.exe)
    # ve terminalde komut vermeyen program klasörlerini at.
    set -l skip 'oculus-runtime|/Intel/Shared Libraries|PhysX|NvDLISR|Nsight Compute|Windows Performance Toolkit|Calibre2|MATLAB/[^/]+/runtime|glzr.io/Zebar|anaconda3/Library/usr/bin|/usr/bin/(site|vendor)_perl|Common Files/Oracle/Java/javapath|libnvvp|Pulsar/resources$'
    set -l clean
    for p in $PATH
        contains -- $p $clean; and continue
        string match -rq -- $skip $p; and continue
        test -d "$p"; and set -a clean $p
    end
    set -gx PATH $clean
    # Windows araçları (scoop, git, python...) MSYS2 araçlarından önce gelsin
    fish_add_path --move --path /c/Users/$USER/scoop/shims /c/Users/$USER/scoop/apps/starship/current

    # Use starship
    function starship_transient_prompt_func
        starship module character
    end
    if test "$TERM" != "linux"
        starship init fish | source
        enable_transience
    end

    # Aliases
    alias clear "printf '\033[2J\033[3J\033[1;1H'"
    alias celar "printf '\033[2J\033[3J\033[1;1H'"
    alias claer "printf '\033[2J\033[3J\033[1;1H'"
    alias pamcan pacman
    if test "$TERM" != "linux"
        alias ls 'eza --icons=auto'
    end
    # Windows'ta sık lazım olanlar
    alias open 'cygstart'
    alias e. 'explorer.exe .'
end

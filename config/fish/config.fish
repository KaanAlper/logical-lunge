# end-4 dots-hyprland .config/fish/config.fish (Windows / MSYS2 uyarlaması)
# Hızlı açılış: login shell değil (MSYS2'nin msys2.fish'i hostname/cygpath çalıştırıyordu, ~130 ms) ve
# starship'in başlatma betiği önbellekten okunur (~140 ms). PATH'i burada kendimiz kuruyoruz.
if status is-interactive
    # No greeting
    set fish_greeting

    # PATH: MSYS2 araçları + Windows PATH'i (tekrarlar, klasör olmayan girişler ve terminalde komut
    # vermeyen program klasörleri atılır; fish her tuşta komutu tüm PATH'te arıyor, MSYS2'de bu pahalı)
    set -l skip 'oculus-runtime|/Intel/Shared Libraries|PhysX|NvDLISR|Nsight Compute|Windows Performance Toolkit|Calibre2|MATLAB/[^/]+/runtime|anaconda3/Library/usr/bin|_perl$|Common Files/Oracle/Java/javapath|libnvvp|Pulsar/resources$|\.exe$'
    set -l clean /c/Users/$USER/scoop/shims "/c/Program Files/LogicalLunge/tools/bin" /c/Users/$USER/scoop/apps/starship/current /ucrt64/bin /usr/local/bin /usr/bin
    for p in $PATH
        contains -- $p $clean; and continue
        string match -rq -- $skip $p; and continue
        set -a clean $p
    end
    set -gx PATH $clean

    # Use starship (başlatma betiği önbellekte; starship güncellenince yenilenir)
    function starship_transient_prompt_func
        starship module character
    end
    if test "$TERM" != "linux"
        set -l bin (command -s starship)
        if test -n "$bin"
            set -l cache ~/.cache/fish/starship-init.fish
            if not test -f $cache; or test $bin -nt $cache
                mkdir -p ~/.cache/fish
                $bin init fish --print-full-init >$cache
            end
            source $cache
            enable_transience
        end
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

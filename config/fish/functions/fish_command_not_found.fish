# Bulunamayan komut: PowerShell tanıyorsa (cls, gci, Get-Process, kendi fonksiyonların ...) orada çalışır. Tek PowerShell
# süreci: komutu bilmiyorsa 127 ile döner ve fish'in her zamanki "komut bulunamadı" iletisi gelir.
function fish_command_not_found
    set -l name $argv[1]
    if string match -qr -- '^[A-Za-z][A-Za-z0-9_.-]*$' "$name"
        set -l ps (command -s pwsh; or command -s powershell.exe; or command -s powershell)
        if test -n "$ps[1]"
            # argümanlar PowerShell tırnağıyla: 'a b', içteki ' ikilenir
            set -l args
            for a in $argv[2..-1]
                if string match -qr -- '^[A-Za-z0-9_./:=@%+,-]+$' "$a"
                    set -a args $a
                else
                    set -a args "'"(string replace -a -- "'" "''" "$a")"'"
                end
            end
            set -l cmd "if (Get-Command -Name '$name' -ErrorAction SilentlyContinue) { $name $args; exit \$LASTEXITCODE } else { exit 127 }"
            MSYS2_ARG_CONV_EXCL='*' $ps[1] -NoLogo -NoProfile -Command $cmd
            set -l st $status
            test $st -ne 127; and return $st
        end
    end
    # fish'in kendi iletisi (bu dosya onunkinin yerine geçtiği için işlevi olmayabilir)
    if functions -q __fish_default_command_not_found_handler
        __fish_default_command_not_found_handler $argv
    else
        printf (_ "fish: Unknown command: %s
") (string escape -- $argv[1]) >&2
    end
end

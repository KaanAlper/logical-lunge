# Performans doğrulaması: Codex'in boşta iş turu ve kare temposu

4 Ekim 2026. Codex'in 4 Ekim'deki performans commit'leri (`2f49b4a`, `c1c40ee`, `9b6bd69`, `24ea67c`) kurulu sistemde ve ekranda ölçüldü. Ölçümlerin hepsi kullanıcı bilgisayardan uzaktayken alındı. Makine: RTX 4080 SUPER, birincil monitör 1920×1080 144 Hz, ikinci monitör 1280×1024, 12 mantıksal çekirdek.

## Özet

| Konu | Sonuç | Durum |
|---|---|---|
| Köşe yuvarlama (`24ea67c`) | Bölge karşılaştırması yuvarlak bölgede hiç tutmuyor. Bölge her olayda yeniden konuyor; boşta bir çekirdekten fazla CPU gidiyor | Düzeltildi: `d5b24a2` |
| Kare temposu (`2f49b4a`) | Zamanlayıcı, ekranda görünen akıcılığı %88'den %81'e düşürüyor; gecikme +1,7 ms | Geri alındı: `2927bae` |
| Dwindle önbelleği, kara kutu eki, tiling eşitlemesi, sysinfo | Doğru | Kaldı |
| Önizleme havuzunu küçültme | Bellek kazancı yok, zararı da yok | Kaldı |

## 1. Köşe döngüsü (kritik)

Codex'in son kurulumundan (18:18) sonra, sistem boştayken alınan ölçüm:

| Süreç | CPU (bir çekirdek = %100) | Uyanma/sn |
|---|---|---|
| lunge (çekirdek) | 50–59 | 18 000 |
| lunge-tiling | 14–15 | 21 000 |
| dwm | 35–39 | 6 500 |
| lunge-wallpaper | 6–10 | 1 700 |

Pencere başına konum olayı sayısı (4 sn): WezTerm 1 094/sn, ChatGPT 121/sn.

**Neden.** `24ea67c`, "bölge zaten bizimki mi" sorusuna bölgenin kutusunu tahmin ederek cevap veriyordu: kutu = köşelerin dikdörtgeni. GDI'de yuvarlak bölgenin kutusu bu dikdörtgenden bir piksel küçüktür. Örneğin `CreateRoundRectRgn(8,0,1009,701)` için kutu `8,0,1008,700` çıkar. Küçük pencerelerde bölgenin türü de değişir (düz ya da boş çıkar). Bu yüzden yuvarlanan hiçbir pencere eşleşmedi ve bölge her olayda yeniden kondu. `SetWindowRgn` da yeni bir konum olayı doğurduğu için kendi kendini besleyen bir döngü oluştu. Tiling her olayı işledi; DWM pencereyi saniyede yüzlerce kez yeniden birleştirdi. Codex'in testi yalnızca köşeli bölgeyi denediği için hata görünmedi.

**Düzeltme (`d5b24a2`).**

- Beklenen tür ve kutu tahmin edilmiyor, aynı bölge GDI'ye üretilip ona soruluyor (`RegionShape`).
- Köşesi yuvarlanamayacak kadar küçük pencere köşeli kesiliyor; hiçbir pencereye boş bölge (görünmez pencere) konmuyor.
- Uygulamanın bölgeyi silmesi gibi kendi bölgesiyle değiştirmesi de kavga sayılıyor; 3 saniyede 4 onarımdan sonra uygulamaya bırakılıyor. Böylece ileride başka bir karşılaştırma hatası olsa bile döngü kurulamıyor.

**Test.**

- 2 şekil × 69 boyut üzerinde GDI karşılaştırması.
- Ayrı bir Windows masaüstünde (`CreateDesktop`) gerçek pencere. Çalışan masaüstünün kancaları ve pencere yöneticisi bu pencereyi görmez. `WM_WINDOWPOSCHANGED` sayılıyor; 20 olayda bölge yeniden konmamalı.
- Eski karşılaştırma geri konunca test kırılıyor: "aynı bölgeyi 4 kez yeniden koydu".

## 2. Kare temposu: DwmFlush ve zamanlayıcı karşılaştırması

`2f49b4a`, kaydırma döngülerinde her karedeki `DwmFlush` yerine dikey boşluğa hizalı, yüksek çözünürlüklü bir zamanlayıcı (`FramePacer`) koymuştu. Kara kutu sayacı bundan sonra "zamanında" demeye başladı. Ama artık ekranı değil, CPU'nun gönderimini ölçüyordu.

Ölçüm aracı `tools/dev/frame-bench`: bir DWM önizlemesi çekirdeğin `PresentClock` zamanıyla kaydırılıyor, ekrana çıkan her kare Desktop Duplication ile yakalanıyor.

| Ölçüm | Yöntem | Akıcı | Tekrar | Atlama | Boşluk | Gecikme ortanca / p95 |
|---|---|---|---|---|---|---|
| Yüksüz, 6+6 tur | DwmFlush | %89,3 | %3,4 | %4,1 | %3,2 | 6,8 / 8,5 ms |
| | Zamanlayıcı | %80,7 | %5,2 | %8,6 | %5,4 | 8,6 / 15,2 ms |
| 12 meşgul süreç, 6+6 | DwmFlush | %95,0 | %0,3 | %1,5 | %3,3 | 6,9 / 8,0 ms |
| | Zamanlayıcı | %84,9 | %2,7 | %6,6 | %5,8 | 7,5 / 9,2 ms |
| Yüksüz, 10+10, kaydedici yüksek öncelikte | DwmFlush | %88,4 | %3,2 | %4,0 | %4,4 | 6,8 / 12,9 ms |
| | Zamanlayıcı | %81,4 | %5,2 | %8,6 | %4,9 | 8,5 / 15,3 ms |

Neden: zamanlayıcı tam DWM'in kareyi topladığı anda uyanıyor, güncelleme bu yüzden iki kare arasında kayıyor. `DwmFlush` ise DWM'in sunumundan hemen sonra döner, bir sonraki güncellemeye tam bir kare pay kalır. `2927bae` döngüleri `DwmFlush`'a, `FrameStats`'ı da o el değiştirmeyi ölçmeye geri aldı. `2f49b4a`'daki kara kutu eki düzeltmeleri kaldı.

Not: yukarıdaki ölçümler, köşe döngüsü sistemi yüklerken alındı. Döngü düzeldikten sonra boşluk oranı düşebilir; iki yöntem aynı koşulda karşılaştırıldı.

## 3. Codex'in diğer değişiklikleri

| Commit | Değişiklik | Değerlendirme |
|---|---|---|
| `9b6bd69` | Bar'ın sysinfo nesnesi süreç taraması yapmıyor | Doğru. Etkisi açılışta (Codex: 33,5 → 3,8 ms). Boşta CPU/RAM sağlayıcıları zaten süreç taramıyordu |
| `9b6bd69` + `24ea67c` | Dwindle önbelleği olaya bağlı; bağlıyken 30 sn, sorgu hatasında 2 sn güvenlik ağı; odak olayı geometri sorgulatmıyor | Doğru |
| `9b6bd69` + `24ea67c` | Kenarlık önizleme havuzu sabit 12 yerine "pencere sayısı + 1" | Bellek kazancı yok: 96 gizli önizleme DWM'in özel belleğini ölçülebilir ölçüde artırmıyor, kaydı 0,1–1 ms sürüyor. Animasyon başlamadan önce yeterli takım hazırlandığı için zararı da yok |
| `9b6bd69` | Yuvarlamadan vazgeçilen pencerede tile kesmesi sürüyor (Discord'un 40 ms'lik taşması) | Doğru; ama `24ea67c`'nin karşılaştırması döngü yaptı (bölüm 1) |
| `c1c40ee` | Tiling: gereksiz `SetWindowPos` ve fullscreen COM çağrısı yok | Doğru; `wm` testleri 16/16 |
| `2f49b4a` | Kara kutu eki kaydın kendisinden; log okuma sınırlı | Doğru |

Testler: çekirdek birim testleri iki sürümde de geçti, `core-tests -RestartOnly` geçti. Rust testleri: `wm` 16/16, kabuk 168/168, duvar kağıdı 12/12.

## 4. Canlı duvar kağıdı

İki monitörde iki ayrı video oynuyor; ikisi de 3840×2160, 60 fps, H.264. Monitörler 1080p ve 1280×1024. Boşta duvar kağıdı süreci %4–10 CPU kullanıyor. Bunun yaklaşık %2,5'i kareyi pencereye aktaran döngü; flip-model swap chain masaüstü simgelerinin altındaki katmanlı pencerede çalışmadığı için bu yol değişmez.

Video çözmenin bedeli (duvar kağıdının Media Foundation yolunun aynısı, 1920×1080 dokuya aktarma, tek video):

| Video | GPU video çözücü | Süreç CPU |
|---|---|---|
| 4K 60 fps | %29,2 | %0,5 |
| 1080p 60 fps | %9,5 | %0,6 |
| 1080p 30 fps | %6,2 | %0,3 |

İkinci monitör kullanıcı tarafından düğmesinden kapatılmış olabilir, ama Windows onu hâlâ etkin görüyor ve video onun için de çözülüyor. DDC/CI ile güç durumu (VCP `D6`) soruldu: açık monitör 54 ms'de "1 = açık" dedi, ikinci monitör hiç cevap vermedi.

## 5. Öneriler (öncelik sırasıyla)

1. **Kurulumdan sonra ölçmek.** `tools/dev/idle-cost.ps1` çalıştırılmalı. Hedef: boşta LL toplamı bir çekirdeğin %1'inin altında, DWM'in boştaki payı döngüden önceki düzeyde. Çekirdekte yapılan her değişiklik kurulduktan sonra bu ölçüm ve `frame-bench` çalıştırılmalı; birim testleri bu iki hatayı yakalamadı.
2. **Duvar kağıdında ekran boyunda kopya.** Video bir kez arka planda, düşük öncelikle ve donanım kodlayıcıyla monitörün çözünürlüğüne indirilip saklanmalı (Media Foundation Transcode/SinkWriter). Bu, GPU çözme yükünü video başına yaklaşık üçte bire indirir. Laptoplarda bu doğrudan pil ve ısı demek.
3. **Kapalı monitörde video durdurmak.** Daha önce DDC/CI'ya "açık" diye cevap veren monitör susarsa ya da `D6` ≥ 4 derse, o ekrandaki video durdurulmalı. Sorgu yaklaşık 50 ms sürüyor; arka planda, en fazla 30 sn'de bir yapılmalı. DDC/CI'yı hiç desteklemeyen monitörde video oynamaya devam eder.
4. **Boştaki uyanmalar.** Döngü kalkınca ölçülmeli. Adaylar:
   - İletişim kutusu güvenlik zamanlayıcısı (500 ms) yalnızca bekleyen kutu varken çalışabilir.
   - Odak bekçisinin 250 ms'lik döngüsü `EVENT_SYSTEM_FOREGROUND`'a bağlanabilir.
   - Köşe yuvarlayıcının 700 ms'lik yoklaması seyrekleştirilebilir. Uygulamanın bölgeyi sıfırlaması da konum olayı doğuruyor; döngünün kendisi bunun kanıtı.
5. **Tiling'in belleği.** 107 MB özel bellek kullanıyor, 30 dakikada büyümedi. Bir pencere yöneticisi için yüksek; heap profili çıkarılmalı.
6. **Sistem zamanlayıcısı 1 ms'de duruyor.** LL'in ikili dosyaları `timeBeginPeriod` ya da `NtSetTimerResolution` çağırmıyor. İsteyen süreç yönetici olarak `powercfg /energy` ile bulunabilir; Media Foundation, tarayıcılar ve Discord olası adaylar.
7. **Bellek, Windows ile karşılaştırma.** Explorer çalışmaya devam ediyor (181 MB özel). LL toplamı yaklaşık 510 MB özel bellek: çekirdek 69, tiling 108, kabuk 102, duvar kağıdı 189, sıcaklık 45. "Windows'tan iyi" hedefi için ya Explorer'ın kabuk yükü kalkmalı ya da LL'in toplamı Explorer'ın görev çubuğu ve başlat bileşenlerinin altına inmeli.

## Kurulum

Düzeltilmiş çekirdek `C:\Temp\ll-rounder-fix-20261004` içinde. Paket Codex'in yerel paketiyle aynı biçimde; yalnızca `lunge.exe` değişti, commit `2927bae`. Yönetici olarak `apply-update.ps1` çalıştırılmalı; ardından `tools/dev/idle-cost.ps1`.

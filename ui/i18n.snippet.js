// Logical Lunge dil katmanı: arayüz Türkçe yazılır; sistem dili Türkçe değilse ekrandaki metinler,
// title ve placeholder'lar i18n.json'dan çevrilir (13 dil; olmayan dil İngilizceye düşer, Arapça sağdan sola).
// Tarih/ay/gün adları window.LL_LOCALE ile (Intl) aynı dilde. Yeni dil / metin: i18n.src.js.
// Kurulumda seçilen dil ve saat prefs.json'da ({"language": "system" | "tr" | "en" ..., "clock": "24" | "12"}):
// dil "system" değilse sistem dilinin yerine geçer; window.LL_HOUR12 saatlerin 12 saatlik (AM/PM) çizilmesi içindir.
(function () {
  function readPrefs(url) {
    try {
      var px = new XMLHttpRequest(); px.open('GET', url, false); px.send();
      if (px.status === 200) return JSON.parse(px.responseText) || null;
    } catch (e) {}
    return null;
  }
  // Tercihler çekirdekten (kullanıcının ayar klasöründe; ayarlar penceresi değiştirir), çekirdek yoksa kurulumun kopyası
  var prefs = readPrefs('http://127.0.0.1:6131/prefs.json') || readPrefs('./prefs.json') || {};
  window.LL_PREFS = prefs;
  window.LL_HOUR12 = prefs.clock === '12';
  // Animasyonlar kapalı (ayarlar): geçişler anında biter. "none" değil: kartları kaldıran animationend olayları yine gelsin
  if (prefs.animations === false) {
    var rm = document.createElement('style');
    rm.textContent = '*,*::before,*::after{animation-duration:.01ms!important;animation-iteration-count:1!important;transition-duration:.01ms!important;scroll-behavior:auto!important}';
    document.head.appendChild(rm);
  }
  var lang = prefs.language && prefs.language !== 'system' ? prefs.language
    : (navigator.languages && navigator.languages[0]) || navigator.language || 'en';
  window.LL_LOCALE = lang;
  var code = lang.slice(0, 2).toLowerCase();
  window.LL_T = function (s) { return s; };
  if (code === 'tr') return;
  var data = null;
  try {
    // Sayfa çizilmeden önce hazır olsun diye eşzamanlı (yerel kabuk sunucusundan, ~1 ms)
    var x = new XMLHttpRequest(); x.open('GET', './i18n.json', false); x.send();
    if (x.status === 200) data = JSON.parse(x.responseText);
  } catch (e) {}
  if (!data) return;
  if (data.langs.indexOf(code) < 0) code = 'en';
  if (code === 'ar') document.documentElement.dir = 'rtl';
  document.documentElement.lang = code;
  var vals = data[code], dict = {}, pats = [];
  function esc(s) { return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&'); }
  data.keys.forEach(function (k, i) {
    if (/\$\d/.test(k)) {
      // "$1 zaten "$2" için..." -> (.+?) grupları; hangi grubun $kaç olduğunu sırayla tut
      var order = (k.match(/\$\d/g) || []).map(function (x) { return +x.charAt(1); });
      pats.push([new RegExp('^' + esc(k).replace(/\\\$\d/g, '(.+?)') + '$'), vals[i], order]);
    }
    else dict[k] = vals[i];
  });
  // Başı sabit metinle başlayan desenler önce: "Açık · 20:00 sonrası" -> Açık · (20:00 sonrası)
  pats.sort(function (a, b) { return (a[0].source.indexOf('^(') === 0) - (b[0].source.indexOf('^(') === 0); });
  function T(s) {
    if (!s) return s;
    var k = s.trim();
    if (!k || !/[A-Za-zÇĞİÖŞÜçğıöşü]/.test(k)) return s;
    var v = dict[k];
    if (v === undefined) {
      for (var i = 0; i < pats.length; i++) {
        var m = k.match(pats[i][0]);
        if (m) {
          v = pats[i][1];
          for (var j = 0; j < pats[i][2].length; j++) v = v.replace('$' + pats[i][2][j], T(m[j + 1])); // iç içe: "Açık · 20:00 sonrası"
          break;
        }
      }
    }
    return v === undefined ? s : s.replace(k, v);
  }
  window.LL_T = T;
  function fixNode(n) {
    if (n.nodeType === 3) { var t = T(n.nodeValue); if (t !== n.nodeValue) n.nodeValue = t; return; }
    if (n.nodeType !== 1) return;
    if (n.classList && n.classList.contains('icon')) return; // ikon adları (Material Symbols ligatürleri)
    ['title', 'placeholder', 'aria-label'].forEach(function (a) {
      var v = n.getAttribute && n.getAttribute(a);
      if (v) { var t = T(v); if (t !== v) n.setAttribute(a, t); }
    });
    for (var c = n.firstChild; c; c = c.nextSibling) fixNode(c);
  }
  new MutationObserver(function (list) {
    for (var i = 0; i < list.length; i++) {
      var m = list[i];
      if (m.type === 'characterData') {
        var p = m.target.parentNode;
        if (!(p && p.classList && p.classList.contains('icon'))) fixNode(m.target);
      } else if (m.type === 'attributes') fixNode(m.target);
      else for (var j = 0; j < m.addedNodes.length; j++) {
        var a = m.addedNodes[j], par = a.parentNode;
        if (a.nodeType === 3 && par && par.classList && par.classList.contains('icon')) continue;
        fixNode(a);
      }
    }
  }).observe(document.documentElement, { subtree: true, childList: true, characterData: true, attributes: true, attributeFilter: ['title', 'placeholder', 'aria-label'] });
})();

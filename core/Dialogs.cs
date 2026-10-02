using System;
using System.Collections.Generic;
using System.Net;
using System.Text;
using System.Threading;
using System.Web.Script.Serialization;

// Sorular: çekirdeğin, betiklerin (lunge.exe --ask) ve pencere yöneticisinin her sorusu kabuğun tek diyaloğunda
// (native_bar/dialog.rs) sorulur; Windows'un ileti kutuları kullanılmaz. POST /dialog soruyu olay akışına koyar
// ({"dialog": {...}}), kabuk diyaloğu açınca /dialog-shown, cevaplayınca /dialog-answer der. Kabuk birkaç saniyede
// "açtım" demezse (masaüstü kapalı, eski bir kabuk) soru "no-ui" ile döner: soran kendi varsayılanına düşer.
static class Dialogs
{
    // kabuğun "açtım" demesi için süre (testler kısaltır)
    public static int ShowWaitMs = 4000;
    public const int ANSWER_WAIT_MS = 30 * 60 * 1000;
    public const int MAX_BUTTONS = 3;
    static readonly string[] KINDS = { "info", "warning", "error", "question" };
    static readonly JavaScriptSerializer json = new JavaScriptSerializer();

    public sealed class Spec
    {
        public string Kind = "question", Title = "", Body = "", Check;
        public string[] Buttons = new string[0];
        public int Default, Cancel = -1;
        public bool Checked;
    }

    sealed class Pending
    {
        public readonly ManualResetEvent Shown = new ManualResetEvent(false), Done = new ManualResetEvent(false);
        public int Button = -1;
        public bool Checked;
    }

    static readonly object gate = new object();
    static readonly Dictionary<long, Pending> pending = new Dictionary<long, Pending>();
    static long next;

    static Dictionary<string, string> Query(string query)
    {
        var d = new Dictionary<string, string>(StringComparer.Ordinal);
        foreach (var part in (query ?? "").Split('&'))
        {
            if (part.Length == 0) continue;
            int eq = part.IndexOf('=');
            string k = eq < 0 ? part : part.Substring(0, eq);
            string v = eq < 0 ? "" : part.Substring(eq + 1);
            d[k] = Uri.UnescapeDataString(v.Replace('+', ' '));
        }
        return d;
    }

    // Saf doğrulama (regression testi bunu dener): null ve hata metni ya da geçerli soru
    public static Spec Parse(string query, out string error)
    {
        error = null;
        var q = Query(query);
        var s = new Spec();
        string v;
        if (q.TryGetValue("kind", out v) && v.Length > 0) s.Kind = v;
        if (Array.IndexOf(KINDS, s.Kind) < 0) { error = "kind"; return null; }
        if (q.TryGetValue("title", out v)) s.Title = v.Trim();
        if (q.TryGetValue("body", out v)) s.Body = v;
        if (s.Title.Length > 200 || s.Body.Length > 4000) { error = "too long"; return null; }
        if (s.Title.Length == 0 && s.Body.Trim().Length == 0) { error = "empty"; return null; }
        var buttons = new List<string>();
        if (q.TryGetValue("buttons", out v))
            foreach (var b in v.Split('|'))
            {
                string t = b.Trim();
                if (t.Length == 0) continue;
                if (t.Length > 40) { error = "button too long"; return null; }
                buttons.Add(t);
            }
        if (buttons.Count == 0 || buttons.Count > MAX_BUTTONS) { error = "buttons"; return null; }
        s.Buttons = buttons.ToArray();
        int n;
        s.Default = q.TryGetValue("default", out v) && int.TryParse(v, out n) && n >= 0 && n < buttons.Count ? n : 0;
        if (q.TryGetValue("cancel", out v) && int.TryParse(v, out n)) s.Cancel = n >= 0 && n < buttons.Count ? n : -1;
        else s.Cancel = buttons.Count == 1 ? 0 : -1;
        if (q.TryGetValue("check", out v) && v.Trim().Length > 0)
        {
            if (v.Length > 200) { error = "check too long"; return null; }
            s.Check = v.Trim();
            s.Checked = q.TryGetValue("checked", out v) && (v == "1" || v == "true");
        }
        return s;
    }

    public static string AnswerJson(int button, bool isChecked, string error)
    {
        var d = new Dictionary<string, object> { { "button", button }, { "checked", isChecked } };
        if (error != null) d["error"] = error;
        return json.Serialize(d);
    }

    // Soruyu kabuğa gönderir ve cevabı bekler (soran thread bekler: HTTP isteği kendi thread'inde)
    public static string Ask(Spec s)
    {
        long id;
        var p = new Pending();
        lock (gate) { id = ++next; pending[id] = p; }
        try
        {
            var d = new Dictionary<string, object>
            {
                { "id", id }, { "kind", s.Kind }, { "title", s.Title }, { "body", s.Body },
                { "buttons", s.Buttons }, { "default", s.Default }, { "cancel", s.Cancel }, { "checked", s.Checked },
            };
            if (s.Check != null) d["check"] = s.Check;
            Toasts.Card(new Dictionary<string, object> { { "dialog", d } });
            if (!p.Shown.WaitOne(ShowWaitMs))
            {
                Slider.Log("soru: kabuk göstermedi (" + s.Title + ")");
                return AnswerJson(-1, s.Checked, "no-ui");
            }
            if (!p.Done.WaitOne(ANSWER_WAIT_MS)) return AnswerJson(-1, s.Checked, "timeout");
            return AnswerJson(p.Button, p.Checked, null);
        }
        finally { lock (gate) pending.Remove(id); }
    }

    public static bool MarkShown(long id)
    {
        Pending p;
        lock (gate) if (!pending.TryGetValue(id, out p)) return false;
        p.Shown.Set();
        return true;
    }

    public static bool Answer(long id, int button, bool isChecked)
    {
        Pending p;
        lock (gate) if (!pending.TryGetValue(id, out p)) return false;
        p.Button = button;
        p.Checked = isChecked;
        p.Shown.Set();
        p.Done.Set();
        return true;
    }

    // lunge.exe --ask --kind question --title T --body B --buttons "Evet|Hayır" [--default 0] [--cancel 1]
    //   [--check "..."] [--checked]: soruyu çalışan çekirdeğe sorar, cevabı tek satır JSON yazar; çıkış kodu
    //   basılan düğmenin sırası (cevapsız: 255). Betikler Read-Host / ileti kutusu yerine bunu kullanır.
    public static int AskCli(string[] args)
    {
        var q = new StringBuilder();
        Action<string, string> add = (k, v) => { if (q.Length > 0) q.Append('&'); q.Append(k).Append('=').Append(Uri.EscapeDataString(v)); };
        for (int i = 1; i < args.Length; i++)
        {
            string a = args[i];
            if (a == "--checked") { add("checked", "1"); continue; }
            if (i + 1 >= args.Length) break;
            switch (a)
            {
                case "--kind": add("kind", args[++i]); break;
                case "--title": add("title", args[++i]); break;
                case "--body": add("body", args[++i]); break;
                case "--buttons": add("buttons", args[++i]); break;
                case "--default": add("default", args[++i]); break;
                case "--cancel": add("cancel", args[++i]); break;
                case "--check": add("check", args[++i]); break;
                default: i++; break;
            }
        }
        string error;
        if (Parse(q.ToString(), out error) == null)
        {
            Console.Out.WriteLine(AnswerJson(-1, false, "invalid: " + error));
            return 255;
        }
        string body;
        try
        {
            var req = (HttpWebRequest)WebRequest.Create("http://127.0.0.1:6131/dialog?" + q);
            req.Method = "POST";
            req.ContentLength = 0;
            req.Proxy = null;
            req.Timeout = ANSWER_WAIT_MS + 60000;
            req.ReadWriteTimeout = ANSWER_WAIT_MS + 60000;
            using (var resp = (HttpWebResponse)req.GetResponse())
            using (var rd = new System.IO.StreamReader(resp.GetResponseStream(), Encoding.UTF8))
                body = rd.ReadToEnd().Trim();
        }
        catch (Exception ex)
        {
            Console.Out.WriteLine(AnswerJson(-1, false, "no-core: " + ex.GetBaseException().Message));
            return 255;
        }
        Console.Out.WriteLine(body);
        try
        {
            var d = json.Deserialize<Dictionary<string, object>>(body);
            int b = d != null && d.ContainsKey("button") ? Convert.ToInt32(d["button"]) : -1;
            return b >= 0 && b < MAX_BUTTONS ? b : 255;
        }
        catch { return 255; }
    }
}

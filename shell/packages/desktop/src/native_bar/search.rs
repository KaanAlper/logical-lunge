//! The Super menu's search, ported from ui/overview.html so both menus give
//! the same results: prefixes, fuzzy app matching (ii Fuzzy.qml), match
//! highlighting, the calculator (ii qalc), actions and the result list.
//! No drawing and no side effects here: an `Act` says what a result does.

// The native Super menu that draws these is being written; until then only the tests use them.
#![allow(dead_code)]

use super::icons::App;

/// ii Config.options.search.prefix
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prefix {
  Default,
  Action,
  App,
  Math,
  Shell,
  Web,
  Clip,
}

impl Prefix {
  pub fn of(query: &str) -> Prefix {
    match query.chars().next() {
      Some('/') => Prefix::Action,
      Some('>') => Prefix::App,
      Some('=') => Prefix::Math,
      Some('$') => Prefix::Shell,
      Some('?') => Prefix::Web,
      Some(';') => Prefix::Clip,
      _ => Prefix::Default,
    }
  }

  /// Material Symbols name drawn in the search shape.
  pub fn icon(self) -> &'static str {
    match self {
      Prefix::Default => "search",
      Prefix::Action => "settings_suggest",
      Prefix::App => "apps",
      Prefix::Math => "calculate",
      Prefix::Shell => "terminal",
      Prefix::Web => "travel_explore",
      Prefix::Clip => "content_paste",
    }
  }
}

const SEARCH_ENGINE: &str = "https://www.google.com/search?q=";

// ------------------------------------------------------------ matching

/// Zero width and other invisible characters (some game names contain them),
/// and the combining dot above that lowercasing "İ" leaves behind.
fn invisible(c: char) -> bool {
  let n = c as u32;
  n == 0xAD || n == 0x307 || (0x200B..=0x200F).contains(&n) || (0x2060..=0x2064).contains(&n) || n == 0xFEFF
}

/// Lowercase for matching. The Turkish dotless and dotted i fold to "i" (in
/// Turkish "Instagram" lowercases to "ınstagram", so "ins" did not match)
/// and invisible characters are dropped. Same as `low` in ui/overview.html.
pub fn fold(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  for c in s.chars() {
    if invisible(c) {
      continue;
    }
    match c {
      'I' | 'İ' | 'ı' => out.push('i'),
      _ => out.extend(c.to_lowercase().filter(|l| !invisible(*l))),
    }
  }
  out
}

fn is_word_break(c: char) -> bool {
  c.is_whitespace() || matches!(c, '-' | '_' | '.' | '(' | ')')
}

/// ii Fuzzy.qml: exact > prefix > every query word starts a word > substring
/// (better at a word start) > initials ("vsc") > letters in order, mostly
/// adjacent. -1: no match.
pub fn fuzzy_score(text: &str, query: &str) -> f64 {
  let t = fold(text);
  let folded = fold(query);
  let q = folded.trim();
  if q.is_empty() {
    return 0.0;
  }
  let tl = t.chars().count() as f64;
  if t == q {
    return 1000.0;
  }
  if t.starts_with(q) {
    return 900.0 - tl;
  }
  let words: Vec<&str> = t.split(is_word_break).filter(|w| !w.is_empty()).collect();
  if q.split_whitespace().all(|qw| words.iter().any(|w| w.starts_with(qw))) {
    return 700.0 - tl;
  }
  if let Some(byte) = t.find(q) {
    let wi = t[..byte].chars().count() as f64;
    let base = if words.iter().any(|w| w.starts_with(q)) { 600.0 } else { 400.0 };
    return base - wi - tl * 0.2;
  }
  let initials: String = words.iter().filter_map(|w| w.chars().next()).collect();
  let compact: String = q.chars().filter(|c| !c.is_whitespace()).collect();
  let ql = q.chars().count();
  if ql >= 2 && initials.starts_with(&compact) {
    return 500.0 - tl;
  }
  if ql < 4 {
    return -1.0;
  }
  let tc: Vec<char> = t.chars().collect();
  let (mut ti, mut contiguous) = (0usize, 0usize);
  for qc in q.chars() {
    let Some(found) = tc.get(ti..).and_then(|rest| rest.iter().position(|&c| c == qc)).map(|p| p + ti) else {
      return -1.0;
    };
    if found == ti {
      contiguous += 1;
    }
    ti = found + 1;
  }
  if (contiguous as f64) < ql as f64 * 0.6 {
    return -1.0;
  }
  200.0 + contiguous as f64 * 10.0 - tl
}

/// Which characters of `content` to underline for `query`: the query's
/// letters, in order, first occurrence each.
pub fn highlight(content: &str, query: &str) -> Vec<bool> {
  let q: Vec<char> = fold(query).chars().filter(|c| !c.is_whitespace()).collect();
  let mut qi = 0;
  content
    .chars()
    .map(|c| {
      let l = fold(&c.to_string());
      let mut lc = l.chars();
      let hit = qi < q.len() && lc.next() == Some(q[qi]) && lc.next().is_none();
      if hit {
        qi += 1;
      }
      hit
    })
    .collect()
}

// ------------------------------------------------------------ calculator

/// ii's qalc on Windows: numbers, + - * / ^ ** % ! ( ), functions and
/// constants. "9" alone is a result too; a lone "e" is an app search. The
/// input is rewritten into a small expression language exactly as
/// ui/overview.html rewrites it into JavaScript, then evaluated with the
/// same rules, so both menus give the same answers.
pub fn eval_math(expr: &str) -> Option<f64> {
  let src = expr.trim().to_lowercase();
  // an expression being typed: its unfinished end is left out ("2*6+" -> 12, "2*" -> 2)
  let src = src.trim_end_matches(|c: char| c.is_whitespace() || "+-*/^×÷(,√".contains(c)).to_string();
  if src.is_empty() || !has_number(&src) {
    return None;
  }
  let src = rewrite_roots(&src.replace('×', "*").replace('÷', "/"));
  let src = if has_call(&src) { src } else { decimal_commas(&src) };
  let out = translate(&src)?;
  let open = out.matches('(').count() as i64 - out.matches(')').count() as i64;
  if open < 0 {
    return None;
  }
  let out = out + &")".repeat(open as usize);
  let v = Parser::new(&out)?.run()?;
  if !v.is_finite() {
    return None;
  }
  // JavaScript's +v.toPrecision(12): hides binary noise (0.1 + 0.2)
  format!("{:.11e}", v).parse().ok()
}

/// a digit or a constant's name; a lone "e" stays an app search
fn has_number(s: &str) -> bool {
  let named = |w: &str| CONSTANTS.iter().any(|(names, _)| names.contains(&w) && w != "e");
  s.chars().any(|c| c.is_ascii_digit() || named(&c.to_string()))
    || s.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')).any(named)
}

/// √9, √(1+3) -> sqrt(9), sqrt((1+3)); any other √ -> sqrt
fn rewrite_roots(s: &str) -> String {
  let c: Vec<char> = s.chars().collect();
  let mut out = String::new();
  let mut i = 0;
  while i < c.len() {
    if c[i] != '√' {
      out.push(c[i]);
      i += 1;
      continue;
    }
    let mut j = i + 1;
    while j < c.len() && c[j].is_whitespace() {
      j += 1;
    }
    // a number (with . or , decimals) or a parenthesis without nested ones
    let end = if j < c.len() && c[j].is_ascii_digit() {
      let mut k = j;
      while k < c.len() && c[k].is_ascii_digit() {
        k += 1;
      }
      if k + 1 < c.len() && (c[k] == '.' || c[k] == ',') && c[k + 1].is_ascii_digit() {
        k += 1;
        while k < c.len() && c[k].is_ascii_digit() {
          k += 1;
        }
      }
      Some(k)
    } else if j < c.len() && c[j] == '(' {
      c[j + 1..].iter().position(|&x| x == '(' || x == ')').filter(|&p| c[j + 1 + p] == ')').map(|p| j + 2 + p)
    } else {
      None
    };
    match end {
      Some(k) => {
        out.push_str("sqrt(");
        out.extend(&c[j..k]);
        out.push(')');
        i = k;
      }
      None => {
        out.push_str("sqrt");
        i += 1;
      }
    }
  }
  out
}

fn is_letter(c: char) -> bool {
  c.is_ascii_lowercase() || "çğıöşü".contains(c)
}

/// a function call ("sqrt(" ...): then commas separate arguments
fn has_call(s: &str) -> bool {
  let c: Vec<char> = s.chars().collect();
  let mut i = 0;
  while i < c.len() {
    if is_letter(c[i]) {
      while i < c.len() && is_letter(c[i]) {
        i += 1;
      }
      let mut j = i;
      while j < c.len() && c[j].is_whitespace() {
        j += 1;
      }
      if j < c.len() && c[j] == '(' {
        return true;
      }
    } else {
      i += 1;
    }
  }
  false
}

/// 3,5 -> 3.5 (left to right, like /(\d),(\d)/g)
fn decimal_commas(s: &str) -> String {
  let c: Vec<char> = s.chars().collect();
  let mut out = String::new();
  let mut i = 0;
  while i < c.len() {
    if c[i].is_ascii_digit() && c.get(i + 1) == Some(&',') && c.get(i + 2).is_some_and(|d| d.is_ascii_digit()) {
      out.push(c[i]);
      out.push('.');
      out.push(c[i + 2]);
      i += 3;
    } else {
      out.push(c[i]);
      i += 1;
    }
  }
  out
}

fn first(a: &[f64]) -> f64 {
  a.first().copied().unwrap_or(f64::NAN)
}

fn factorial(a: &[f64]) -> f64 {
  let n = first(a);
  if !(0.0..=170.0).contains(&n) || n.fract() != 0.0 {
    return f64::NAN;
  }
  (2..=n as u32).map(f64::from).product()
}

type MathFn = fn(&[f64]) -> f64;

/// The calculator's functions, in the web menu's order (MATH_FN): the names
/// that can be typed and what they compute. Name lookup, the "sqrt(" hint and
/// evaluation all read this one table.
static FUNCTIONS: &[(&[&str], MathFn)] = &[
  (&["sqrt", "karekok", "karekök"], |a| first(a).sqrt()),
  (&["cbrt"], |a| first(a).cbrt()),
  (&["abs"], |a| first(a).abs()),
  (&["exp"], |a| first(a).exp()),
  (&["ln"], |a| first(a).ln()),
  (&["log", "log10"], |a| first(a).log10()),
  (&["log2"], |a| first(a).log2()),
  (&["sin"], |a| first(a).sin()),
  (&["cos"], |a| first(a).cos()),
  (&["tan"], |a| first(a).tan()),
  (&["asin"], |a| first(a).asin()),
  (&["acos"], |a| first(a).acos()),
  (&["atan"], |a| first(a).atan()),
  (&["sinh"], |a| first(a).sinh()),
  (&["cosh"], |a| first(a).cosh()),
  (&["tanh"], |a| first(a).tanh()),
  (&["floor"], |a| first(a).floor()),
  (&["ceil"], |a| first(a).ceil()),
  // JavaScript rounds halves up: round(-2.5) = -2
  (&["round"], |a| (first(a) + 0.5).floor()),
  (&["min"], |a| a.iter().copied().fold(f64::INFINITY, f64::min)),
  (&["max"], |a| a.iter().copied().fold(f64::NEG_INFINITY, f64::max)),
  (&["pow"], |a| first(a).powf(a.get(1).copied().unwrap_or(f64::NAN))),
  // the ! operator; not offered as a hint (as in the web menu)
  (&["fact"], factorial),
];

/// Constants: typed names and their value in the translated expression.
static CONSTANTS: &[(&[&str], &str)] =
  &[(&["pi", "π"], "PI"), (&["e"], "E"), (&["tau"], "(2*PI)"), (&["phi"], "((1+sqrt_(5))/2)")];

/// A function's name in the translated expression: its first name plus "_",
/// so digits typed after it ("log 2") cannot turn it into another name.
fn function_token(id: &str) -> Option<String> {
  FUNCTIONS.iter().find(|(names, _)| names.contains(&id)).map(|(names, _)| format!("{}_", names[0]))
}

fn function_by_token(token: &str) -> Option<MathFn> {
  let name = token.strip_suffix('_')?;
  FUNCTIONS.iter().find(|(names, _)| names[0] == name).map(|(_, f)| *f)
}

fn constant(id: &str) -> Option<&'static str> {
  CONSTANTS.iter().find(|(names, _)| names.contains(&id)).map(|(_, v)| *v)
}

/// The first function name that starts with `typed` (the calculator hint).
fn hint_function(typed: &str) -> Option<&'static str> {
  FUNCTIONS
    .iter()
    .filter(|(names, _)| names[0] != "fact")
    .flat_map(|(names, _)| names.iter().copied())
    .find(|n| n.starts_with(typed))
}

/// The web menu's tokenizer: numbers, names, operators; implicit
/// multiplication (2pi, (2)3), postfix ! and %. None: not an expression.
fn translate(e: &str) -> Option<String> {
  let c: Vec<char> = e.chars().collect();
  let mut out = String::new();
  let mut last = String::new();
  let mut i = 0;
  while i < c.len() {
    let ch = c[i];
    if ch.is_whitespace() {
      i += 1;
    } else if ch.is_ascii_digit() {
      let start = i;
      while i < c.len() && c[i].is_ascii_digit() {
        i += 1;
      }
      if i + 1 < c.len() && c[i] == '.' && c[i + 1].is_ascii_digit() {
        i += 1;
        while i < c.len() && c[i].is_ascii_digit() {
          i += 1;
        }
      }
      if i < c.len() && c[i] == 'e' {
        let mut k = i + 1;
        if k < c.len() && (c[k] == '+' || c[k] == '-') {
          k += 1;
        }
        if k < c.len() && c[k].is_ascii_digit() {
          while k < c.len() && c[k].is_ascii_digit() {
            k += 1;
          }
          i = k;
        }
      }
      let num: String = c[start..i].iter().collect();
      if last.ends_with(')') {
        out.push('*');
      }
      out.push_str(&num);
      last = num;
    } else if is_letter(ch) || ch == 'π' {
      let start = i;
      i += 1;
      while i < c.len() && (is_letter(c[i]) || c[i].is_ascii_digit()) {
        i += 1;
      }
      let id: String = c[start..i].iter().collect();
      let pre = if out.ends_with(|x: char| x.is_ascii_digit() || x == ')') { "*" } else { "" };
      if let Some(f) = function_token(&id) {
        // a function name must be followed by "(" ("sqrt tau" is not a call)
        if c[i..].iter().find(|x| !x.is_whitespace()) != Some(&'(') {
          return None;
        }
        out.push_str(pre);
        out.push_str(&f);
        last = id;
      } else if let Some(k) = constant(&id) {
        out.push_str(pre);
        out.push_str(k);
        last = ")".into();
      } else {
        return None;
      }
    } else if ch == '*' && c.get(i + 1) == Some(&'*') {
      out.push_str("**");
      last = "**".into();
      i += 2;
    } else if "+-*/%^(),!".contains(ch) {
      i += 1;
      match ch {
        '^' => out.push_str("**"),
        '!' => {
          // the last number or parenthesis without nested ones: 5!, (2+3)!
          let start = trailing_operand(&out)?;
          let operand = out.split_off(start);
          out.push_str("fact_(");
          out.push_str(&operand);
          out.push(')');
        }
        '%' => {
          // postfix percent unless a number or "(" follows: 50% -> 0.5, 10%3 -> 1
          let rest = c[i..].iter().skip_while(|x| x.is_whitespace()).next();
          let percent = out.ends_with(|x: char| x.is_ascii_digit() || x == ')')
            && !rest.is_some_and(|x| x.is_ascii_digit() || *x == '(');
          out.push_str(if percent { "/100" } else { "%" });
        }
        _ => out.push(ch),
      }
      last = ch.to_string();
    } else {
      return None;
    }
  }
  Some(out)
}

/// Byte index where the trailing number or simple parenthesis of `out` starts.
fn trailing_operand(out: &str) -> Option<usize> {
  let b = out.as_bytes();
  let n = b.len();
  if n == 0 {
    return None;
  }
  if b[n - 1] == b')' {
    let open = out[..n - 1].rfind(|x: char| x == '(' || x == ')')?;
    return (b[open] == b'(').then_some(open);
  }
  if !b[n - 1].is_ascii_digit() {
    return None;
  }
  let mut i = n;
  while i > 0 && b[i - 1].is_ascii_digit() {
    i -= 1;
  }
  if i >= 2 && b[i - 1] == b'.' && b[i - 2].is_ascii_digit() {
    i -= 1;
    while i > 0 && b[i - 1].is_ascii_digit() {
      i -= 1;
    }
  }
  Some(i)
}

#[derive(Clone, Debug, PartialEq)]
enum Tok {
  Num(f64),
  Name(String),
  Op(&'static str),
}

/// JavaScript's rules for the expression `translate` makes: ** is right
/// associative and a unary sign may not stand before it (-2**2 is an error),
/// "--" is an error, a call needs a known function.
struct Parser {
  toks: Vec<Tok>,
  pos: usize,
}

impl Parser {
  fn new(s: &str) -> Option<Parser> {
    let b = s.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0;
    while i < b.len() {
      let ch = b[i];
      if ch.is_ascii_digit() {
        let start = i;
        while i < b.len() && (b[i].is_ascii_digit() || b[i] == b'.') {
          i += 1;
        }
        if i < b.len() && b[i] == b'e' {
          let mut k = i + 1;
          if k < b.len() && (b[k] == b'+' || b[k] == b'-') {
            k += 1;
          }
          if k < b.len() && b[k].is_ascii_digit() {
            while k < b.len() && b[k].is_ascii_digit() {
              k += 1;
            }
            i = k;
          }
        }
        // JavaScript reads "2e" / "1.2.3" as errors too
        if i < b.len() && (b[i].is_ascii_alphabetic() || b[i] == b'_') {
          return None;
        }
        toks.push(Tok::Num(s[start..i].parse().ok()?));
      } else if ch.is_ascii_alphabetic() || ch == b'_' {
        let start = i;
        while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
          i += 1;
        }
        toks.push(Tok::Name(s[start..i].to_string()));
      } else {
        let two = s.get(i..i + 2).unwrap_or("");
        let op: &'static str = match (two, ch) {
          ("**", _) => "**",
          ("--", _) | ("++", _) => return None,
          (_, b'+') => "+",
          (_, b'-') => "-",
          (_, b'*') => "*",
          (_, b'/') => "/",
          (_, b'%') => "%",
          (_, b'(') => "(",
          (_, b')') => ")",
          (_, b',') => ",",
          _ => return None,
        };
        i += op.len();
        toks.push(Tok::Op(op));
      }
    }
    Some(Parser { toks, pos: 0 })
  }

  fn run(mut self) -> Option<f64> {
    let v = self.add()?;
    (self.pos == self.toks.len()).then_some(v)
  }

  fn peek_op(&self) -> Option<&'static str> {
    match self.toks.get(self.pos) {
      Some(Tok::Op(o)) => Some(*o),
      _ => None,
    }
  }

  fn eat(&mut self, op: &str) -> bool {
    if self.peek_op() == Some(op) {
      self.pos += 1;
      true
    } else {
      false
    }
  }

  fn add(&mut self) -> Option<f64> {
    let mut v = self.mul()?;
    loop {
      if self.eat("+") {
        v += self.mul()?;
      } else if self.eat("-") {
        v -= self.mul()?;
      } else {
        return Some(v);
      }
    }
  }

  fn mul(&mut self) -> Option<f64> {
    let mut v = self.unary()?;
    loop {
      if self.eat("*") {
        v *= self.unary()?;
      } else if self.eat("/") {
        v /= self.unary()?;
      } else if self.eat("%") {
        v %= self.unary()?;
      } else {
        return Some(v);
      }
    }
  }

  fn unary(&mut self) -> Option<f64> {
    if self.eat("-") {
      return Some(-self.signed_operand()?);
    }
    if self.eat("+") {
      return self.signed_operand();
    }
    self.power()
  }

  /// after a sign: another sign, or a base that is not raised to a power
  fn signed_operand(&mut self) -> Option<f64> {
    if matches!(self.peek_op(), Some("-") | Some("+")) {
      return self.unary();
    }
    let v = self.primary()?;
    if self.peek_op() == Some("**") {
      return None;
    }
    Some(v)
  }

  fn power(&mut self) -> Option<f64> {
    let base = self.primary()?;
    if self.eat("**") {
      let exp = self.unary()?;
      return Some(base.powf(exp));
    }
    Some(base)
  }

  fn primary(&mut self) -> Option<f64> {
    match self.toks.get(self.pos).cloned()? {
      Tok::Num(n) => {
        self.pos += 1;
        Some(n)
      }
      Tok::Op("(") => {
        self.pos += 1;
        let v = self.add()?;
        self.eat(")").then_some(v)
      }
      Tok::Name(name) => {
        self.pos += 1;
        match name.as_str() {
          "PI" => return Some(std::f64::consts::PI),
          "E" => return Some(std::f64::consts::E),
          _ => {}
        }
        if !self.eat("(") {
          return None;
        }
        let mut args = Vec::new();
        if !self.eat(")") {
          loop {
            args.push(self.add()?);
            if self.eat(")") {
              break;
            }
            if !self.eat(",") {
              return None;
            }
          }
        }
        Some(function_by_token(&name)?(&args))
      }
      Tok::Op(_) => None,
    }
  }
}

/// A number the way JavaScript prints it: 0.3, 1024, 1e+21, 1.5e-7.
pub fn format_number(v: f64) -> String {
  if v == 0.0 {
    return "0".into();
  }
  let exp_form = format!("{:e}", v);
  let (mantissa, exp) = exp_form.split_once('e').unwrap_or((exp_form.as_str(), "0"));
  let exp: i32 = exp.parse().unwrap_or(0);
  // JavaScript prints 1e-6 .. 1e21 (exclusive) in plain notation
  if (-6..21).contains(&exp) {
    format!("{}", v)
  } else {
    format!("{}e{}{}", mantissa, if exp >= 0 { "+" } else { "" }, exp)
  }
}

// ------------------------------------------------------------ results

/// ii's /actions (Config.options.search.actions).
pub const ACTIONS: &[(&str, &str)] = &[
  ("dark", "Karanlık/aydınlık tema"),
  ("lock", "Ekranı kilitle"),
  ("sleep", "Uyku"),
  ("logout", "Oturumu kapat"),
  ("restart", "Yeniden başlat"),
  ("shutdown", "Kapat"),
  ("reload", "Pencere yöneticisi ayarlarını yenile"),
  ("apps", "Uygulama listesini yenile"),
];

/// A clipboard history entry (`lunge.exe --clip-list`).
#[derive(Clone, Debug, Default, serde::Deserialize)]
pub struct Clip {
  #[serde(default)]
  pub id: String,
  #[serde(default)]
  pub kind: String,
  #[serde(default)]
  pub text: String,
  #[serde(default)]
  pub lines: u32,
  #[serde(default)]
  pub time: i64,
  #[serde(default)]
  pub thumb: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Glyph {
  /// the app list entry's icon (index into the app list)
  App(usize),
  /// a clipboard image (data URL)
  Image(String),
  Material(&'static str),
  /// a large character ("=" for the calculator)
  Big(&'static str),
}

/// What choosing a result does.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
  None,
  /// open an app list entry (`explorer <path>`)
  Launch(String),
  /// scripts\run.ps1 <mode> <text>: "run", "term" (terminal command), "url"
  Script(&'static str, String),
  Copy(String),
  /// put a clipboard history entry back (`lunge.exe --clip-set <id>`)
  ClipSet(String),
  /// replace the query (the calculator hint)
  Query(String),
  /// one of `ACTIONS`
  Action(&'static str),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
  pub key: String,
  /// "Uygulama", "Eylem" ... (translated when drawn)
  pub kind: &'static str,
  /// small text after the kind: a line count, a time, an action's description
  pub sub: String,
  pub name: String,
  pub glyph: Glyph,
  pub verb: &'static str,
  pub mono: bool,
  /// choosing it keeps the menu open
  pub stay: bool,
  /// underline these letters of `name` (the app search term)
  pub highlight: Option<String>,
  pub clip_id: Option<String>,
  pub act: Act,
}

impl Item {
  fn new(key: impl Into<String>, kind: &'static str, name: impl Into<String>, glyph: Glyph, verb: &'static str, act: Act) -> Item {
    Item {
      key: key.into(),
      kind,
      sub: String::new(),
      name: name.into(),
      glyph,
      verb,
      mono: false,
      stay: false,
      highlight: None,
      clip_id: None,
      act,
    }
  }
}

/// JavaScript's encodeURIComponent.
pub fn encode_uri_component(s: &str) -> String {
  let mut out = String::new();
  for b in s.bytes() {
    if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
      out.push(b as char);
    } else {
      out.push_str(&format!("%{:02X}", b));
    }
  }
  out
}

fn web_search(term: &str) -> Act {
  Act::Script("url", format!("{}{}", SEARCH_ENGINE, encode_uri_component(term)))
}

/// The result list for `query` (ui/overview.html `results`). `time` formats
/// a clipboard entry's time (the locale's clock).
pub fn results(query: &str, apps: &[App], clips: &[Clip], time: &dyn Fn(i64) -> String) -> Vec<Item> {
  let mut out = Vec::new();
  if query.is_empty() {
    return out;
  }
  let rest = &query[query.chars().next().map(char::len_utf8).unwrap_or(0)..];
  match Prefix::of(query) {
    Prefix::Clip => {
      let term = rest.trim();
      let list: Vec<&Clip> = clips
        .iter()
        .filter(|c| if c.kind == "image" { term.is_empty() } else { term.is_empty() || fuzzy_score(&c.text, term) >= 0.0 })
        .collect();
      if list.is_empty() {
        let name = if term.is_empty() { "Pano geçmişi boş" } else { "Eşleşen kayıt yok" };
        let mut it = Item::new("cbnone", "Pano", name, Glyph::Material("content_paste"), "", Act::None);
        it.stay = true;
        out.push(it);
      }
      for c in list.into_iter().take(40) {
        let first = c.text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        let image = c.kind == "image";
        let name: String = if image { "Görüntü".into() } else { first.chars().take(140).collect() };
        let glyph = match (&c.thumb, image) {
          (Some(t), true) => Glyph::Image(t.clone()),
          _ => Glyph::Material("content_paste"),
        };
        let mut it = Item::new(format!("cb{}", c.id), "Pano", name, glyph, "Kopyala", Act::ClipSet(c.id.clone()));
        let lines = if c.kind == "text" && c.lines > 1 { format!("{} satır", c.lines) } else { String::new() };
        it.sub = [lines, time(c.time)].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" · ");
        it.mono = c.kind == "text" && first.contains(|x: char| "{};=<>".contains(x));
        it.clip_id = Some(c.id.clone());
        out.push(it);
      }
      return out;
    }
    Prefix::Action => {
      let a = rest.to_lowercase();
      let word = a.split(' ').next().unwrap_or("");
      for &(name, desc) in ACTIONS.iter().filter(|(n, _)| n.starts_with(word)) {
        let mut it = Item::new(format!("a{}", name), "Eylem", format!("/{}", name), Glyph::Material("settings_suggest"), "Çalıştır", Act::Action(name));
        it.sub = desc.into();
        out.push(it);
      }
      return out;
    }
    Prefix::Shell => {
      let cmd = rest.trim();
      let act = if cmd.is_empty() { Act::None } else { Act::Script("term", cmd.into()) };
      let mut it = Item::new("sh", "Komut", if cmd.is_empty() { "komut yaz…" } else { cmd }, Glyph::Material("terminal"), "Çalıştır", act);
      it.mono = true;
      out.push(it);
      return out;
    }
    Prefix::Web => {
      let s = rest.trim();
      out.push(Item::new("web", "Web araması", s, Glyph::Material("travel_explore"), "Ara", web_search(s)));
      return out;
    }
    Prefix::Default | Prefix::App | Prefix::Math => {}
  }

  let prefix = Prefix::of(query);
  let (app_only, math_only) = (prefix == Prefix::App, prefix == Prefix::Math);
  let term = if app_only || math_only { rest.trim() } else { query.trim() };

  if let Some(m) = eval_math(term) {
    let text = format_number(m);
    out.push(Item::new("math", "Hesap makinesi", text.clone(), Glyph::Big("="), "Kopyala", Act::Copy(text)));
  } else if term.chars().count() >= 2 {
    // introduce the calculator: "sqrt", "sin", "hesap" show it too; choosing it starts the expression
    let lt = fold(term);
    let function = hint_function(&lt);
    let keyword = ["hesap", "calc", "matemat", "math", "="].iter().any(|k| lt.starts_with(k));
    if function.is_some() || keyword {
      let (name, start) = match function {
        Some(f) => (format!("{}( … )", f), format!("{}(", f)),
        None => ("2+2 · sqrt(9) · 5! · 2^10 · %20".to_string(), String::new()),
      };
      let mut it = Item::new("mathhint", "Hesap makinesi", name, Glyph::Big("="), "Yaz", Act::Query(start));
      it.stay = true;
      out.push(it);
    }
  }
  if math_only {
    return out;
  }

  if !term.is_empty() {
    let lt = fold(term);
    let mut scored: Vec<(usize, f64)> = apps
      .iter()
      .enumerate()
      .map(|(i, a)| {
        let s = if a.alias.as_deref().is_some_and(|al| fold(al) == lt) {
          2000.0
        } else {
          fuzzy_score(&a.name, term).max(a.also.as_deref().map(|al| fuzzy_score(al, term)).unwrap_or(-1.0))
        };
        (i, s)
      })
      .filter(|&(_, s)| s >= 0.0)
      .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    for (i, _) in scored.into_iter().take(15) {
      let a = &apps[i];
      let mut it = Item::new(format!("app{}", a.path), "Uygulama", a.name.clone(), Glyph::App(i), "Aç", Act::Launch(a.path.clone()));
      it.highlight = Some(term.to_string());
      out.push(it);
    }
  }
  if !app_only && !term.is_empty() {
    let mut run = Item::new("run", "Komut çalıştır", term, Glyph::Material("terminal"), "Çalıştır", Act::Script("run", term.into()));
    run.mono = true;
    out.push(run);
    out.push(Item::new("web", "Web araması", term, Glyph::Material("travel_explore"), "Ara", web_search(term)));
  }
  out
}

#[cfg(test)]
mod tests {
  use super::*;

  fn app(name: &str) -> App {
    serde_json::from_value(serde_json::json!({ "name": name, "path": format!("shell:AppsFolder\\{}", name) })).unwrap()
  }

  fn math(s: &str) -> Option<String> {
    eval_math(s).map(format_number)
  }

  #[test]
  fn fuzzy_tiers() {
    assert_eq!(fuzzy_score("Firefox", "firefox"), 1000.0);
    assert_eq!(fuzzy_score("Firefox", "fire"), 900.0 - 7.0);
    assert_eq!(fuzzy_score("Ekran Klavyesi", "ekr kla"), 700.0 - 14.0);
    assert!(fuzzy_score("Visual Studio Code", "vsc") >= 500.0 - 18.0);
    assert_eq!(fuzzy_score("Firefox", "xyzqq"), -1.0);
    assert_eq!(fuzzy_score("abc", "zz"), -1.0);
    // substring: better at a word start
    assert!(fuzzy_score("Google Chrome", "chrome") > fuzzy_score("Xchromey", "chrome"));
  }

  #[test]
  fn turkish_i_and_invisible_characters() {
    assert_eq!(fuzzy_score("Instagram", "ins"), 900.0 - 9.0);
    assert_eq!(fuzzy_score("İnternet", "int"), 900.0 - 8.0);
    assert_eq!(fuzzy_score("ılık", "ilik"), 1000.0);
    let zw = char::from_u32(0x200B).unwrap();
    assert_eq!(fuzzy_score(&format!("4{}{}42 Still Free", zw, zw), "442"), 900.0 - 14.0);
  }

  #[test]
  fn highlight_letters_in_order() {
    let h = highlight("Visual Studio Code", "vsc");
    let marked: String = "Visual Studio Code".chars().zip(h).filter(|(_, m)| *m).map(|(c, _)| c).collect();
    assert_eq!(marked, "VsC");
    let h = highlight("Instagram", "in");
    assert_eq!(&h[..2], &[true, true]);
  }

  /// One example per rule; web_parity compares every short input and many
  /// long ones with the web menu.
  #[test]
  fn calculator_rules() {
    let cases: &[(&str, Option<&str>)] = &[
      ("9", Some("9")),                    // a number alone is a result
      ("e", None),                         // a lone "e" is an app search
      ("sqrt(9", Some("3")),               // open parentheses are closed
      ("-2^2", None),                      // JavaScript: a sign before ** is an error
      ("50% + 1", Some("1.5")),            // % after a number is percent, before a number modulo
      ("(2+3)!", Some("120")),             // ! takes the last number or parenthesis
      ("2pi", Some("6.28318530718")),      // implicit multiplication, 12 significant digits
      ("3,5+1", Some("4.5")),              // comma decimals when there is no call
      ("2,pi", None),                      // otherwise commas only separate arguments
      ("sqrt tau", None),                  // a function needs its parenthesis
      ("max--", None),                     // ++ / -- are assignments, not calculations
    ];
    for &(input, want) in cases {
      assert_eq!(math(input).as_deref(), want, "{:?}", input);
    }
  }

  #[test]
  fn numbers_print_like_javascript() {
    assert_eq!(format_number(0.0), "0");
    assert_eq!(format_number(1e20), "100000000000000000000");
    assert_eq!(format_number(1e21), "1e+21");
    assert_eq!(format_number(0.000001), "0.000001");
    assert_eq!(format_number(1.5e-7), "1.5e-7");
    assert_eq!(format_number(-42.5), "-42.5");
  }

  #[test]
  fn apps_that_do_not_match_are_left_out() {
    let apps = vec![app("2XKO"), app("Firefox"), app("Instagram")];
    let r = results("xyzqq", &apps, &[], &|_| String::new());
    assert_eq!(r.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["run", "web"]);
    let r = results("ins", &apps, &[], &|_| String::new());
    assert_eq!(r[0].name, "Instagram");
    assert_eq!(r[0].act, Act::Launch("shell:AppsFolder\\Instagram".into()));
  }

  #[test]
  fn prefixes() {
    let none: &[App] = &[];
    let t = |_| String::new();
    let r = results("/da", none, &[], &t);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].act, Act::Action("dark"));
    let r = results("$ dir", none, &[], &t);
    assert_eq!(r[0].act, Act::Script("term", "dir".into()));
    let r = results("?a b", none, &[], &t);
    assert_eq!(r[0].act, Act::Script("url", "https://www.google.com/search?q=a%20b".into()));
    let r = results("=2*3", none, &[], &t);
    assert_eq!(r.len(), 1);
    assert_eq!(r[0].name, "6");
    let r = results("sq", none, &[], &t);
    assert_eq!(r[0].key, "mathhint");
    assert_eq!(r[0].act, Act::Query("sqrt(".into()));
    let r = results(">fire", &[app("Firefox")], &[], &t);
    assert_eq!(r.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(), ["appshell:AppsFolder\\Firefox"]);
  }

  /// Compares with the web menu's own functions on hundreds of expressions
  /// and the real app list (data from scratchpad parity-gen.mjs, which runs
  /// ui/overview.html's tryMath / fuzzyScore in node). Runs only when
  /// LL_PARITY_JSON names that file.
  #[test]
  fn web_parity() {
    let Some(path) = std::env::var_os("LL_PARITY_JSON") else { return };
    let data: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut bad = Vec::new();
    for m in data["math"].as_array().unwrap() {
      let input = m["input"].as_str().unwrap();
      let want = m["out"].as_str().map(str::to_string);
      let got = math(input);
      if got != want {
        bad.push(format!("hesap {:?}: web {:?}, native {:?}", input, want, got));
      }
    }
    for f in data["fuzzy"].as_array().unwrap() {
      let (text, query) = (f["text"].as_str().unwrap(), f["query"].as_str().unwrap());
      let want = f["score"].as_f64().unwrap();
      let got = fuzzy_score(text, query);
      if (got - want).abs() > 1e-9 {
        bad.push(format!("eşleşme {:?} / {:?}: web {}, native {}", text, query, want, got));
      }
    }
    assert!(bad.is_empty(), "{} fark:
{}", bad.len(), bad.iter().take(40).cloned().collect::<Vec<_>>().join("
"));
  }

  #[test]
  fn clipboard() {
    let clips = vec![
      Clip { id: "1".into(), kind: "text".into(), text: "\n  let x = 1;\nsecond".into(), lines: 3, time: 0, thumb: None },
      Clip { id: "2".into(), kind: "image".into(), thumb: Some("data:x".into()), ..Default::default() },
    ];
    let t = |_| "10:00".to_string();
    let r = results(";", &[], &clips, &t);
    assert_eq!(r.len(), 2);
    assert_eq!(r[0].name, "let x = 1;");
    assert_eq!(r[0].sub, "3 satır · 10:00");
    assert!(r[0].mono);
    assert_eq!(r[1].glyph, Glyph::Image("data:x".into()));
    let r = results(";zzzz", &[], &clips, &t);
    assert_eq!(r[0].key, "cbnone");
    assert!(r[0].stay);
  }
}

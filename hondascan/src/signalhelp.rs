//! Honda's own explanation of a signal, from the HDS help pages (db table `help`, keys `4R_<group>_<symbol>`). Port of
//! hondascan/signalhelp.py: the group is learned per system from the prefixes under which at least a few of its symbols have
//! a page; pages are cleaned lightly (banner, repeated title, unit hints, diagram legends go; lines are joined; common
//! abbreviations are spelled out where they first appear); a page that says the value is not right while the vehicle is
//! stopped marks the signal `invalid_stopped`.
use crate::db::Database;
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, OnceLock};

pub const MIN_HITS: usize = 5;
pub const MAX_CHARS: usize = 700;
const BANNER: &str = "PARAMETER SPECIFICATION";
const SPELL: [(&str, &str); 17] = [
    ("A/T", "automatic transmission"), ("AT", "automatic transmission"), ("ECU", "electronic control unit"), ("PCM", "engine computer"),
    ("TCM", "transmission computer"), ("TP", "throttle position"), ("MAP", "manifold pressure"), ("MAF", "air flow"),
    ("ECT", "coolant temperature"), ("IAT", "intake air temperature"), ("A/F", "air-fuel"), ("VTC", "variable valve timing"),
    ("VSA", "stability assist"), ("ABS", "anti-lock brakes"), ("SRS", "airbags"), ("EPS", "power steering"), ("DTC", "trouble code"),
];

struct Res {
    hint: Regex,
    skip: Regex,
    stopped: Regex,
    leading_units: Regex,
    legend: Regex,
    ws: Regex,
}

fn res() -> &'static Res {
    static R: OnceLock<Res> = OnceLock::new();
    R.get_or_init(|| Res {
        hint: Regex::new(r"^[(\[].{0,40}[)\]]\.?$").unwrap(), // "($XXXX)", "(%)", "[HEX]", "(0,1,2,3)", "[Description]"
        skip: Regex::new(r"(?i)^(PARAMETER TYPE|THE SOURCE)\s*:").unwrap(),
        stopped: Regex::new(r"(?i)(not (be )?(correct|valid|accurate|reliable)|inaccurate|differs?|different)[^.]{0,80}\b(stop|stops|stopped|stopping|standstill|stationary|at rest)\b").unwrap(),
        leading_units: Regex::new(r"^(\([^)]{1,12}\)\s*)+").unwrap(), // "(Pa) (kPa) (MPa) ..." before the text
        legend: Regex::new(r"\s[A-Z]: [^.]{3,60}? [B-Z]: ").unwrap(), // a diagram key: "A: Sensor B: Circuit C: ..."
        ws: Regex::new(r"\s+").unwrap(),
    })
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// Python's `re.sub(rf'(?<![\w/]){short}(?![\w/])', f'{long} ({short})', body, count=1)`
fn spell_first(body: &str, short: &str, long: &str) -> String {
    let mut from = 0;
    while let Some(p) = body[from..].find(short) {
        let at = from + p;
        let before = body[..at].chars().next_back();
        let after = body[at + short.len()..].chars().next();
        if !before.is_some_and(|c| is_word(c) || c == '/') && !after.is_some_and(|c| is_word(c) || c == '/') {
            return format!("{}{long} ({short}){}", &body[..at], &body[at + short.len()..]);
        }
        from = at + short.chars().next().map_or(1, char::len_utf8);
    }
    body.to_string()
}

pub fn clean(text: &str) -> String {
    let r = res();
    let lines: Vec<&str> = text.split('\n').map(str::trim).skip(1).collect();
    let mut out: Vec<String> = vec![];
    let mut skip_next = false;
    for ln in lines {
        if skip_next {
            skip_next = false;
        } else if ln.is_empty() || ln.to_uppercase() == BANNER {
            continue;
        } else if r.skip.is_match(ln) {
            skip_next = ln.to_uppercase().starts_with("PARAMETER TYPE"); // its value is the next line
            if ln.to_uppercase().starts_with("THE SOURCE") {
                out.push("Source:".into());
            }
        } else if r.hint.is_match(ln) {
            continue;
        } else {
            out.push(ln.to_string());
        }
    }
    if out.len() > 1 {
        out.remove(0); // the title repeats the signal's name
    }
    let mut body = r.ws.replace_all(&out.join(" "), " ").trim().to_string();
    body = r.leading_units.replace(&body, "").into_owned();
    if let Some(m) = r.legend.find(&body) {
        body = body[..m.start()].trim_end().to_string();
    }
    if body.chars().count() > MAX_CHARS {
        let head: String = body.chars().take(MAX_CHARS).collect();
        body = match head.rfind(". ") {
            Some(cut) if head[..cut].chars().count() > 200 => head[..cut + 1].to_string(),
            _ => format!("{}…", head.trim_end()),
        };
    }
    for (short, long) in SPELL {
        body = spell_first(&body, short, long);
    }
    body
}

/// What a help page becomes: its cleaned body (empty: it says nothing worth showing) and whether the body says the value is not
/// right while the vehicle is stopped. `build-db` stores this for every page that carries a parameter specification.
pub fn analyse(page: &str) -> (String, bool) {
    let body = clean(page);
    let stopped = !body.is_empty() && res().stopped.is_match(&body);
    (body, stopped)
}

/// One symbol's help in one system: (the help page's body, does the body say the signal stops).
type SymbolHelp = HashMap<String, (String, bool)>;

/// The keys of the help pages that carry a parameter specification (`signal_help`, built by `build-db`), in the order the old
/// scan of `help` gave them, and the same keys as a set. The bodies stay in the database until a system asks for them.
struct Pages { keys: Vec<String>, set: HashSet<String> }

fn load_pages(db: &Database) -> Pages {
    let keys: Vec<String> = db.query("SELECT key FROM signal_help ORDER BY rowid", [], |r| r.get(0));
    let set = keys.iter().cloned().collect();
    Pages { keys, set }
}

pub struct SignalHelp {
    db: Arc<Database>,
    pages: OnceLock<Pages>,
    by_system: Mutex<HashMap<String, Arc<SymbolHelp>>>,
}

impl SignalHelp {
    pub fn new(db: Arc<Database>) -> SignalHelp {
        SignalHelp { db, pages: OnceLock::new(), by_system: Mutex::new(HashMap::new()) }
    }

    fn pages(&self) -> &Pages { self.pages.get_or_init(|| load_pages(&self.db)) }

    /// Does a help prefix ('4R_CMETER_', 'ICM_FI_') name the system (CAN_METER, CANFI)?
    pub fn names(prefix: &str, system: &str) -> bool {
        let hint = system.strip_prefix("CAN").unwrap_or(system);
        let hint = hint.strip_prefix('_').unwrap_or(hint);
        prefix.trim_matches('_').split('_').any(|t| hint == t || hint == t.strip_prefix("CAN").unwrap_or(t) || hint == t.strip_prefix('C').unwrap_or(t))
    }

    /// The help prefixes ('4R_FI_') under which at least `MIN_HITS` of the system's symbols have a page and which name the system, the
    /// most hits first. One pass over the keys: a key ends with `_SYMBOL` when the text after one of its underscores is a symbol of
    /// the system. The hits are then taken in the order the old symbol-by-symbol scan met them, so ties keep that order.
    pub fn prefixes(&self, system: &str, symbols: &[String]) -> Vec<String> {
        let want: HashMap<&str, usize> = symbols.iter().enumerate().map(|(i, s)| (s.as_str(), i)).collect();
        let mut seen: Vec<(usize, usize, &str)> = vec![]; // (symbol, key, prefix)
        for (n, k) in self.pages().keys.iter().enumerate() {
            for (i, _) in k.match_indices('_') {
                if let Some(&sym) = want.get(&k[i + 1..]) {
                    if Self::names(&k[..=i], system) { seen.push((sym, n, &k[..=i])); }
                }
            }
        }
        seen.sort_unstable_by_key(|h| (h.0, h.1));
        let mut hits: Vec<(&str, usize)> = vec![]; // insertion-ordered
        for (_, _, p) in seen {
            match hits.iter_mut().find(|(x, _)| *x == p) {
                Some(h) => h.1 += 1,
                None => hits.push((p, 1)),
            }
        }
        hits.sort_by_key(|h| std::cmp::Reverse(h.1)); // stable: ties keep first-seen order
        hits.into_iter().filter(|h| h.1 >= MIN_HITS).map(|h| h.0.to_string()).collect()
    }

    /// {symbol: (cleaned help, invalid while stopped)} for the symbols of `system` that have a page.
    pub fn for_system(&self, system: &str) -> Arc<SymbolHelp> {
        if let Some(h) = self.by_system.lock().unwrap().get(system) {
            return h.clone();
        }
        let symbols: Vec<String> = self.db.query("SELECT DISTINCT symbol FROM can_signal_parts WHERE system=?", [system], |r| r.get(0));
        let prefixes = self.prefixes(system, &symbols);
        let pages = self.pages();
        // the first prefix under which a symbol has a page is its page, whether or not the cleaned page says anything
        let found: Vec<(&String, String)> = symbols.iter().filter_map(|s| prefixes.iter().map(|p| format!("{p}{s}")).find(|k| pages.set.contains(k)).map(|k| (s, k))).collect();
        let mut bodies: HashMap<String, (String, bool)> = HashMap::new();
        for chunk in found.chunks(400) {
            let marks = vec!["?"; chunk.len()].join(",");
            let rows = self.db.query(&format!("SELECT key, body, stopped FROM signal_help WHERE key IN ({marks})"),
                rusqlite::params_from_iter(chunk.iter().map(|f| &f.1)), |r| Ok((r.get::<_, String>(0)?, (r.get::<_, String>(1)?, r.get::<_, i64>(2)? != 0))));
            bodies.extend(rows);
        }
        let out: SymbolHelp = found.into_iter().filter_map(|(s, k)| bodies.remove(&k).filter(|b| !b.0.is_empty()).map(|b| (s.clone(), b))).collect();
        let out = Arc::new(out);
        self.by_system.lock().unwrap().insert(system.into(), out.clone());
        out
    }
}

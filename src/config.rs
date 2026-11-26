//! `%APPDATA%\logirate\config.toml`: everything about which app gets which rate lives here.

use crate::foreground::Foreground;
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;

pub const VALID_RATES: [u32; 7] = [125, 250, 500, 1000, 2000, 4000, 8000];

/// Explanation written at the top of every saved config.
pub const DOC: &str = r#"# logirate config. It's reloaded automatically when you save this file.
# The logirate window and `logirate add/remove` rewrite this file (this help text is kept).
#
# rate: 125, 250, 500, 1000, 2000, 4000, 8000, or "onboard".
#   "onboard" hands control back to the mouse's own saved profile (exactly what it does without logirate).
#   Any number switches the mouse to temporary host mode with that rate. Nothing is written to the mouse;
#   unplugging it or switching it off always brings back the onboard profile.
#
# default:     used when no rule matches.  default_dpi: optional DPI for a numeric default.
# poll_ms:     how often to look at the foreground app, in milliseconds (100-5000).
#
# Rules are checked top to bottom; the first match wins.
#
# [[rule]]
# name    = "CS2"                 # optional label for the log
# process = "cs2.exe"             # one name or a list: ["a.exe", "b*.exe"]. * and ? are wildcards.
#                                 # A pattern with a \ is matched against the full path instead:
#                                 # 'D:\Games\*'  matches everything installed there.
# title   = "Counter-Strike"      # optional: the window title must contain this (case-insensitive)
# when    = "focused"             # "focused" (default): only while it's the foreground app
#                                 # "running": as long as the process runs, focused or not
# rate    = 8000
# dpi     = 800                   # optional: DPI while this rule is active (default: keep the profile's DPI)

"#;

fn toml_str(s: &str) -> String {
    toml::Value::String(s.to_string()).to_string()
}

fn rate_value(t: Target) -> String {
    match t {
        Target::Onboard => "\"onboard\"".into(),
        Target::Host { hz, .. } => hz.to_string(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    Onboard,
    Host { hz: u32, dpi: Option<u16> },
}

impl std::fmt::Display for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Target::Onboard => write!(f, "onboard profile"),
            Target::Host { hz, dpi: None } => write!(f, "{hz} Hz"),
            Target::Host { hz, dpi: Some(d) } => write!(f, "{hz} Hz, {d} DPI"),
        }
    }
}

/// Parses "8000", "8000hz" or "onboard".
pub fn parse_rate(s: &str) -> Result<Option<u32>> {
    let s = s.trim().to_ascii_lowercase();
    if s == "onboard" {
        return Ok(None);
    }
    let hz: u32 = s.trim_end_matches("hz").trim().parse().with_context(|| format!("bad rate '{s}'"))?;
    if !VALID_RATES.contains(&hz) {
        bail!("rate must be one of {VALID_RATES:?} or \"onboard\", not {hz}");
    }
    Ok(Some(hz))
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawRate {
    Hz(u32),
    Word(String),
}

impl RawRate {
    fn hz(&self) -> Result<Option<u32>> {
        match self {
            RawRate::Hz(hz) => parse_rate(&hz.to_string()),
            RawRate::Word(w) => parse_rate(w),
        }
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum OneOrMany {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    name: Option<String>,
    process: Option<OneOrMany>,
    title: Option<String>,
    when: Option<String>,
    rate: RawRate,
    dpi: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawConfig {
    default: Option<RawRate>,
    default_dpi: Option<u16>,
    poll_ms: Option<u64>,
    #[serde(default, rename = "rule")]
    rules: Vec<RawRule>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Focused,
    Running,
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub name: String,
    pub processes: Vec<String>,
    pub title: Option<String>,
    pub when: When,
    pub target: Target,
}

impl Rule {
    fn matches(&self, fg: Option<&Foreground>, running: &HashSet<String>) -> bool {
        match self.when {
            When::Focused => {
                let Some(fg) = fg else { return false };
                let proc_ok = self.processes.is_empty()
                    || self.processes.iter().any(|p| {
                        if p.contains('\\') || p.contains('/') { glob(p, &fg.path.replace('/', "\\")) } else { glob(p, &fg.exe) }
                    });
                let title_ok = self.title.as_ref().is_none_or(|t| fg.title.contains(t.as_str()));
                proc_ok && title_ok
            }
            When::Running => self.processes.iter().any(|p| running.iter().any(|exe| glob(p, exe))),
        }
    }
}

#[derive(Debug, Clone)]
pub struct Config {
    pub default: Target,
    pub poll_ms: u64,
    pub rules: Vec<Rule>,
}

impl Default for Config {
    fn default() -> Self {
        Config { default: Target::Onboard, poll_ms: 500, rules: Vec::new() }
    }
}

impl Config {
    /// Serializes the config (with the help text on top).
    pub fn to_toml(&self) -> String {
        let mut s = String::from(DOC);
        s += &format!("default = {}\n", rate_value(self.default));
        if let Target::Host { dpi: Some(d), .. } = self.default {
            s += &format!("default_dpi = {d}\n");
        }
        s += &format!("poll_ms = {}\n", self.poll_ms);
        for r in &self.rules {
            s += "\n[[rule]]\n";
            s += &format!("name = {}\n", toml_str(&r.name));
            match r.processes.as_slice() {
                [] => {}
                [one] => s += &format!("process = {}\n", toml_str(one)),
                many => s += &format!("process = [{}]\n", many.iter().map(|p| toml_str(p)).collect::<Vec<_>>().join(", ")),
            }
            if let Some(t) = &r.title {
                s += &format!("title = {}\n", toml_str(t));
            }
            if r.when == When::Running {
                s += "when = \"running\"\n";
            }
            s += &format!("rate = {}\n", rate_value(r.target));
            if let Target::Host { dpi: Some(d), .. } = r.target {
                s += &format!("dpi = {d}\n");
            }
        }
        s
    }

    /// Validates by re-parsing, then writes the config file.
    pub fn save(&self) -> Result<()> {
        let text = self.to_toml();
        Self::parse(&text).context("refusing to save an invalid config")?;
        let path = Self::path();
        std::fs::create_dir_all(path.parent().unwrap())?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)?;
        Ok(())
    }

    pub fn path() -> PathBuf {
        crate::paths::config_file()
    }

    /// Creates the default config if there is none, and returns its text.
    pub fn read_or_create() -> Result<String> {
        let path = Self::path();
        if !path.exists() {
            Config::default().save()?;
        }
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))
    }

    pub fn load() -> Result<Config> {
        Self::parse(&Self::read_or_create()?).with_context(|| format!("in {}", Self::path().display()))
    }

    pub fn parse(text: &str) -> Result<Config> {
        let raw: RawConfig = toml::from_str(text)?;
        let mut rules = Vec::new();
        for (i, r) in raw.rules.into_iter().enumerate() {
            let processes: Vec<String> = match r.process {
                None => Vec::new(),
                Some(OneOrMany::One(p)) => vec![p],
                Some(OneOrMany::Many(v)) => v,
            }
            .into_iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| !p.is_empty())
            .collect();
            let title = r.title.map(|t| t.to_lowercase()).filter(|t| !t.is_empty());
            let when = match r.when.as_deref().map(str::to_ascii_lowercase).as_deref() {
                None | Some("focused") | Some("foreground") => When::Focused,
                Some("running") => When::Running,
                Some(other) => bail!("rule {}: when must be \"focused\" or \"running\", not \"{other}\"", i + 1),
            };
            if processes.is_empty() && (title.is_none() || when == When::Running) {
                bail!("rule {}: needs a process (or, for focused rules, a title)", i + 1);
            }
            if when == When::Running {
                if title.is_some() {
                    bail!("rule {}: a window title only works with when = \"focused\"", i + 1);
                }
                if processes.iter().any(|p| p.contains('\\') || p.contains('/')) {
                    bail!("rule {}: running rules match program names only, not paths", i + 1);
                }
            }
            let target = match r.rate.hz().with_context(|| format!("rule {}", i + 1))? {
                None => Target::Onboard,
                Some(hz) => Target::Host { hz, dpi: r.dpi },
            };
            let name = r.name.unwrap_or_else(|| {
                if processes.is_empty() { format!("title \"{}\"", title.clone().unwrap_or_default()) } else { processes.join(", ") }
            });
            rules.push(Rule { name, processes, title, when, target });
        }
        let default = match raw.default.map(|r| r.hz()).transpose()?.flatten() {
            None => Target::Onboard,
            Some(hz) => Target::Host { hz, dpi: raw.default_dpi },
        };
        Ok(Config { default, poll_ms: raw.poll_ms.unwrap_or(500).clamp(100, 5000), rules })
    }

    pub fn needs_process_list(&self) -> bool {
        self.rules.iter().any(|r| r.when == When::Running)
    }

    /// The target for the current situation, and the rule that picked it.
    pub fn target_for(&self, fg: Option<&Foreground>, running: &HashSet<String>) -> (Target, Option<&Rule>) {
        match self.rules.iter().find(|r| r.matches(fg, running)) {
            Some(r) => (r.target, Some(r)),
            None => (self.default, None),
        }
    }
}

/// Case-sensitive glob with `*` and `?` (callers lower-case both sides).
pub fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    let (mut pi, mut ti, mut star, mut mark) = (0, 0, None, 0);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some(pi);
            mark = ti;
            pi += 1;
        } else if let Some(s) = star {
            pi = s + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|&c| c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fg(path: &str, title: &str) -> Foreground {
        let path = path.to_lowercase();
        Foreground { exe: path.rsplit('\\').next().unwrap().into(), path, title: title.to_lowercase() }
    }

    #[test]
    fn default_config_has_no_rules() {
        let c = Config::parse(&Config::default().to_toml()).unwrap();
        assert!(c.rules.is_empty());
        assert_eq!(c.default, Target::Onboard);
    }

    #[test]
    fn round_trip() {
        let src = r#"
            default = 1000
            default_dpi = 1600
            [[rule]]
            name = "Shooters \"fast\""
            process = ["cs2.exe", 'd:\games\*']
            title = "counter"
            rate = 8000
            dpi = 800
            [[rule]]
            process = "obs64.exe"
            when = "running"
            rate = "onboard"
        "#;
        let a = Config::parse(src).unwrap();
        let b = Config::parse(&a.to_toml()).unwrap();
        assert_eq!(a.default, b.default);
        assert_eq!(a.rules.len(), b.rules.len());
        for (x, y) in a.rules.iter().zip(&b.rules) {
            assert_eq!((&x.name, &x.processes, &x.title, x.when, x.target), (&y.name, &y.processes, &y.title, y.when, y.target));
        }
    }

    #[test]
    fn matching() {
        let c = Config::parse(
            r#"
            default = 1000
            [[rule]]
            process = ["cs2.exe", "valorant*.exe"]
            rate = 8000
            dpi = 800
            [[rule]]
            process = 'd:\games\*'
            rate = "onboard"
            [[rule]]
            title = "youtube"
            rate = 500
            [[rule]]
            process = "obs64.exe"
            when = "running"
            rate = 2000
            "#,
        )
        .unwrap();
        let none = HashSet::new();
        let t = |p: &str, title: &str| c.target_for(Some(&fg(p, title)), &none).0;
        assert_eq!(t(r"C:\Steam\cs2.exe", "Counter-Strike 2"), Target::Host { hz: 8000, dpi: Some(800) });
        assert_eq!(t(r"C:\Riot\VALORANT-Win64-Shipping.exe", ""), Target::Host { hz: 8000, dpi: Some(800) });
        assert_eq!(t(r"D:\Games\Witcher\witcher3.exe", ""), Target::Onboard);
        assert_eq!(t(r"C:\Chrome\chrome.exe", "Lofi - YouTube"), Target::Host { hz: 500, dpi: None });
        assert_eq!(t(r"C:\Windows\explorer.exe", ""), Target::Host { hz: 1000, dpi: None });
        let running: HashSet<String> = ["obs64.exe".to_string()].into();
        assert_eq!(c.target_for(Some(&fg(r"C:\x\notepad.exe", "")), &running).0, Target::Host { hz: 2000, dpi: None });
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Config::parse("[[rule]]\nprocess='x.exe'\nrate=3000").is_err());
        assert!(Config::parse("[[rule]]\nrate=1000").is_err());
        assert!(Config::parse("[[rule]]\nprocess='x.exe'\nrate=1000\nwhen='sometimes'").is_err());
        assert!(Config::parse("typo = 1").is_err());
        assert!(Config::parse("[[rule]]\nprocess='d:\\games\\*'\nwhen='running'\nrate=1000").is_err());
        assert!(Config::parse("[[rule]]\nprocess='obs64.exe'\ntitle='x'\nwhen='running'\nrate=1000").is_err());
        assert_eq!(parse_rate("1000Hz").unwrap(), Some(1000));
    }

    #[test]
    fn globbing() {
        assert!(glob("cs2.exe", "cs2.exe"));
        assert!(glob("*.exe", "cs2.exe"));
        assert!(glob("valorant*", "valorant-win64-shipping.exe"));
        assert!(glob("cs?.exe", "cs2.exe"));
        assert!(!glob("cs2.exe", "cs2.exe.bak"));
    }
}

use anyhow::{Result, anyhow, bail};
use logirate::config::{Config, Rule, Target, When, parse_rate};
use logirate::hidpp::{Device, Mode, feature};

const HELP: &str = "logirate: per-app polling rate for Logitech mice (no G HUB)

usage:
  logirate info                        show the mouse, its rates, mode and DPI
  logirate set <hz|onboard> [dpi]      set a rate now, or hand back to the onboard profile
  logirate rules                       list the configured rules
  logirate add <process|path> <hz|onboard> [--dpi N] [--title TEXT] [--running] [--name NAME]
                                       add a rule (process may use * and ?; a pattern with \\ matches the full path)
  logirate remove <number|name>        remove a rule (numbers as shown by `rules`)
  logirate default <hz|onboard> [dpi]  set what applies when no rule matches
  logirate watch                       follow the foreground app using the config (Ctrl+C to stop)
  logirate config                      print the config file path (creates it if missing)

logirated.exe runs the same watcher without a console and logs to
%LOCALAPPDATA%\\logirate\\logirate.log.";

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run(args: &[String]) -> Result<()> {
    match args.first().map(String::as_str) {
        Some("info") => info(),
        Some("set") => {
            let rate = args.get(1).ok_or_else(|| anyhow!("usage: logirate set <hz|onboard> [dpi]"))?;
            set(target(rate, args.get(2))?)
        }
        Some("rules") => rules(),
        Some("add") => add(&args[1..]),
        Some("remove") => remove(args.get(1).ok_or_else(|| anyhow!("usage: logirate remove <number|name>"))?),
        Some("default") => {
            let rate = args.get(1).ok_or_else(|| anyhow!("usage: logirate default <hz|onboard> [dpi]"))?;
            let mut c = Config::load()?;
            c.default = target(rate, args.get(2))?;
            c.save()?;
            println!("default: {}", c.default);
            Ok(())
        }
        Some("watch") => logirate::watch::run(&mut |m| println!("{} {m}", stamp())),
        Some("config") => Config::load().map(|_| println!("{}", Config::path().display())),
        Some("raw") => raw(&args[1..]),
        _ => {
            println!("{HELP}");
            Ok(())
        }
    }
}

fn target(rate: &str, dpi: Option<&String>) -> Result<Target> {
    let dpi = dpi.map(|d| d.parse::<u16>()).transpose().map_err(|_| anyhow!("dpi must be a number"))?;
    Ok(match parse_rate(rate)? {
        None => Target::Onboard,
        Some(hz) => Target::Host { hz, dpi },
    })
}

fn info() -> Result<()> {
    let mut dev = Device::open()?;
    let (major, minor) = dev.protocol()?;
    println!("receiver : {}", dev.receiver());
    let name = dev.name().unwrap_or_else(|e| format!("(name unavailable: {e})"));
    println!("device   : #{} {name}", dev.index);
    println!("protocol : HID++ {major}.{minor}");
    for (id, label) in [
        (feature::EXT_REPORT_RATE, "0x8061 extended report rate"),
        (feature::REPORT_RATE, "0x8060 report rate"),
        (feature::ONBOARD_PROFILES, "0x8100 onboard profiles"),
        (feature::ADJUSTABLE_DPI, "0x2201 adjustable DPI"),
        (feature::EXT_ADJUSTABLE_DPI, "0x2202 extended adjustable DPI"),
    ] {
        println!("feature  : {label}: {}", if dev.has(id)? { "yes" } else { "no" });
    }
    println!("rates    : {:?} Hz", dev.supported_rates()?);
    println!("current  : {} Hz", dev.rate()?);
    match dev.mode()? {
        Some(m) => println!("mode     : {m}{}", if m == Mode::Onboard { " (mouse uses its saved profile)" } else { "" }),
        None => println!("mode     : (no onboard profiles)"),
    }
    match dev.dpi() {
        Ok(d) => println!("dpi      : {d}{}", d.lod.map(|l| format!(" (lift-off setting {l})")).unwrap_or_default()),
        Err(e) => println!("dpi      : ({e})"),
    }
    if std::env::args().any(|a| a == "--raw") && dev.has(feature::EXT_ADJUSTABLE_DPI)? {
        println!("raw 2202 : {:02x?}", dev.dpi_raw()?);
    }
    Ok(())
}

fn set(t: Target) -> Result<()> {
    let mut dev = Device::open()?;
    match t {
        Target::Onboard => dev.use_onboard()?,
        Target::Host { hz, dpi } => dev.set_host(hz, dpi)?,
    }
    let mode = dev.mode()?.map(|m| m.to_string()).unwrap_or("-".into());
    let dpi = dev.dpi().map(|d| d.to_string()).unwrap_or("-".into());
    println!("now: {} Hz, {dpi} DPI, mode {mode}", dev.rate()?);
    Ok(())
}

fn rules() -> Result<()> {
    let c = Config::load()?;
    println!("config : {}", Config::path().display());
    println!("default: {}", c.default);
    if c.rules.is_empty() {
        println!("no rules yet. Example: logirate add cs2.exe 8000");
    }
    for (i, r) in c.rules.iter().enumerate() {
        let mut what = r.processes.join(", ");
        if let Some(t) = &r.title {
            what += &format!("{}title contains \"{t}\"", if what.is_empty() { "" } else { " + " });
        }
        let when = if r.when == When::Running { "while running" } else { "while focused" };
        println!("{:>3}. {}: {what} {when} -> {}", i + 1, r.name, r.target);
    }
    Ok(())
}

fn add(args: &[String]) -> Result<()> {
    let (mut positional, mut dpi, mut title, mut name, mut when) = (Vec::new(), None, None, None, When::Focused);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--dpi" => dpi = Some(it.next().ok_or_else(|| anyhow!("--dpi needs a value"))?.parse::<u16>()?),
            "--title" => title = Some(it.next().ok_or_else(|| anyhow!("--title needs a value"))?.to_lowercase()),
            "--name" => name = Some(it.next().ok_or_else(|| anyhow!("--name needs a value"))?.clone()),
            "--running" => when = When::Running,
            other if other.starts_with("--") => bail!("unknown option {other}"),
            other => positional.push(other.to_string()),
        }
    }
    let [process, rate] = positional.as_slice() else {
        bail!("usage: logirate add <process|path> <hz|onboard> [--dpi N] [--title TEXT] [--running] [--name NAME]");
    };
    let target = match parse_rate(rate)? {
        None => Target::Onboard,
        Some(hz) => Target::Host { hz, dpi },
    };
    let process = process.to_lowercase();
    let mut c = Config::load()?;
    c.rules.push(Rule { name: name.unwrap_or_else(|| process.clone()), processes: vec![process], title, when, target });
    c.save()?;
    rules()
}

fn remove(which: &str) -> Result<()> {
    let mut c = Config::load()?;
    let idx = match which.parse::<usize>() {
        Ok(n) if n >= 1 && n <= c.rules.len() => n - 1,
        _ => c
            .rules
            .iter()
            .position(|r| r.name.eq_ignore_ascii_case(which))
            .ok_or_else(|| anyhow!("no rule \"{which}\" (see `logirate rules`)"))?,
    };
    let r = c.rules.remove(idx);
    c.save()?;
    println!("removed: {}", r.name);
    rules()
}

/// `logirate raw <feature hex> <function> [param bytes hex...]`: diagnostics, prints the reply.
fn raw(args: &[String]) -> Result<()> {
    let [feat, func, params @ ..] = args else { bail!("usage: logirate raw <feature hex> <function> [bytes hex...]") };
    let feat = u16::from_str_radix(feat.trim_start_matches("0x"), 16)?;
    let func: u8 = func.parse()?;
    let params = params.iter().map(|p| u8::from_str_radix(p.trim_start_matches("0x"), 16)).collect::<Result<Vec<_>, _>>()?;
    let mut dev = Device::open()?;
    println!("{feat:04x} fn{func} {params:02x?} -> {:02x?}", dev.raw(feat, func, &params)?);
    Ok(())
}

fn stamp() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    format!("[{:02}:{:02}:{:02} UTC]", secs / 3600 % 24, secs / 60 % 60, secs % 60)
}

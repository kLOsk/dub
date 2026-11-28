//! `dub rig-coach` — reads the rig checklist aloud, one item at a time.
//!
//! `docs/RIG-CHECKLIST.md` is written to be read at a desk and run at
//! the turntables, which is two places. The coach speaks each item, the
//! DJ does it and answers with one key, and the run is written down as
//! it goes. It also watches the stall folder, so a dump that lands
//! during an item is named on that item without anyone looking.
//!
//! The checklist stays the only list: the coach parses it, and a test
//! here fails if an item in it stops parsing.
//!
//! With `--record` (what `make rig` passes) each item is also a take:
//! `dub capture` of every input of the interface, from the moment the
//! item comes up to its answer, named in the run file beside the verdict.
//! A gesture done once at the decks — a lift, a stop, a scratch — can then
//! be played back through the loop cable as often as a check needs it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context, Result};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal;

const USAGE: &str = "usage: dub rig-coach [--tag hands|robot|loop] [--section N] \
     [--checklist PATH] [--out DIR] [--fresh] [--mute] [--list] \
     [--record [--device NAME] [--takes DIR]]";

const DEFAULT_CHECKLIST: &str = "docs/RIG-CHECKLIST.md";
const DEFAULT_OUT_DIR: &str = "docs/rig-runs";
/// Gitignored: a take is the interface's every input as 32-bit float,
/// about 70 MB a minute on the SL 3.
const DEFAULT_TAKES_DIR: &str = "testdata/rig-takes";
/// The longest one take records: an item left up over lunch stops here.
const TAKE_CAP_SECS: &str = "1800";
/// How long a capture gets to close its file after the answer.
const TAKE_STOP_WAIT: Duration = Duration::from_secs(5);

/// Words per minute. Fast enough for a long item not to drag, slow
/// enough to follow with a record running.
const SPEECH_RATE: &str = "185";

/// One checkbox of the checklist.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Item {
    /// `section.n`, counting every item of the section whatever its
    /// tag, so an id means the same line to the DJ and to the agent.
    id: String,
    section: u32,
    section_title: String,
    /// `hands`, `robot` or `loop`; empty when the item carries none.
    tag: String,
    /// The item's text with its markdown, joined onto one line.
    text: String,
}

fn section_heading(line: &str) -> Option<(u32, String)> {
    let rest = line.strip_prefix("## ")?;
    let (number, title) = rest.split_once(". ")?;
    Some((number.parse().ok()?, title.trim().to_string()))
}

/// Split `` `tag …` rest `` into the tag's first word and the rest.
fn split_tag(text: &str) -> (String, &str) {
    let Some(inner) = text.strip_prefix('`') else {
        return (String::new(), text);
    };
    let Some((span, rest)) = inner.split_once('`') else {
        return (String::new(), text);
    };
    let tag = span.split_whitespace().next().unwrap_or_default();
    (tag.to_string(), rest.trim_start())
}

fn parse_checklist(markdown: &str) -> Vec<Item> {
    let mut items: Vec<Item> = Vec::new();
    let mut section: Option<(u32, String)> = None;
    let mut in_section = 0_u32;
    let mut open = false;

    for line in markdown.lines() {
        if line.starts_with("## ") {
            section = section_heading(line);
            in_section = 0;
            open = false;
            continue;
        }
        let Some((number, title)) = &section else {
            continue;
        };
        let checkbox = line
            .strip_prefix("- [ ] ")
            .or_else(|| line.strip_prefix("- [x] "));
        if let Some(body) = checkbox {
            in_section += 1;
            let (tag, text) = split_tag(body.trim());
            items.push(Item {
                id: format!("{number}.{in_section}"),
                section: *number,
                section_title: title.clone(),
                tag,
                text: text.to_string(),
            });
            open = true;
        } else if open && line.starts_with(' ') && !line.trim().is_empty() {
            if let Some(item) = items.last_mut() {
                item.text.push(' ');
                item.text.push_str(line.trim());
            }
        } else {
            open = false;
        }
    }
    items
}

/// The item as text on a screen: markdown marks gone, symbols kept.
fn plain(text: &str) -> String {
    text.chars().filter(|c| !matches!(c, '*' | '`')).collect()
}

/// The item as `say` should read it. The checklist is full of signs
/// that are read with the eye: an arrow for "expect", a menu path, a
/// minus that is not a hyphen.
fn spoken(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    for c in plain(text).chars() {
        match c {
            '→' => out.push_str(". Expect: "),
            '▸' | '·' => out.push_str(", "),
            '−' => out.push_str("minus "),
            '×' => out.push_str("times "),
            '÷' => out.push_str("divided by "),
            '″' => out.push_str(" inch"),
            '⅔' => out.push_str("two thirds"),
            '¾' => out.push_str("three quarters"),
            '♩' | '<' | '>' | '○' => {}
            '—' => out.push_str(", "),
            other => out.push(other),
        }
    }
    let squeezed = out.split_whitespace().collect::<Vec<_>>().join(" ");
    squeezed.replace(" .", ".").replace(" ,", ",")
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verdict {
    Pass,
    Fail,
    Skip,
}

impl Verdict {
    fn word(self) -> &'static str {
        match self {
            Self::Pass => "PASS",
            Self::Fail => "FAIL",
            Self::Skip => "SKIP",
        }
    }

    fn from_word(word: &str) -> Option<Self> {
        match word {
            "PASS" => Some(Self::Pass),
            "FAIL" => Some(Self::Fail),
            "SKIP" => Some(Self::Skip),
            _ => None,
        }
    }

    fn said(self) -> &'static str {
        match self {
            Self::Pass => "Pass.",
            Self::Fail => "Fail.",
            Self::Skip => "Skipped.",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    verdict: Verdict,
    /// What happened, in the DJ's words. Asked for on a FAIL.
    note: String,
    /// Stall dumps that appeared while the item was up.
    stalls: Vec<String>,
    /// The item's take, when the run records one.
    take: Option<String>,
}

/// What a run file says about itself, above its answers.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct RunHeader {
    started: String,
    commit: String,
    machine: String,
    interface: String,
    tester: String,
    tag: String,
}

const NOTE_OPEN: &str = " — *";
const STALL_MARK: &str = " · stall: ";
const TAKE_MARK: &str = " · take: ";

fn answer_line(item: &Item, answer: &Answer) -> String {
    let mut line = format!(
        "- {} · {} · {}",
        answer.verdict.word(),
        item.id,
        plain(&item.text)
    );
    for stall in &answer.stalls {
        let _ = write!(line, "{STALL_MARK}{stall}");
    }
    if let Some(take) = &answer.take {
        let _ = write!(line, "{TAKE_MARK}{take}");
    }
    if !answer.note.is_empty() {
        let _ = write!(line, "{NOTE_OPEN}{}*", answer.note);
    }
    line
}

fn render_run(header: &RunHeader, items: &[Item], answers: &BTreeMap<String, Answer>) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# Rig run {}", header.started);
    let _ = writeln!(out);
    let _ = writeln!(out, "| | |");
    let _ = writeln!(out, "|---|---|");
    let _ = writeln!(out, "| Commit | {} |", header.commit);
    let _ = writeln!(out, "| Machine · macOS | {} |", header.machine);
    let _ = writeln!(out, "| Interface | {} |", header.interface);
    let _ = writeln!(out, "| Tester | {} |", header.tester);
    let _ = writeln!(out, "| Items | `{}` |", header.tag);

    let count = |v: Verdict| answers.values().filter(|a| a.verdict == v).count();
    let _ = writeln!(
        out,
        "| Result | {} pass · {} fail · {} skipped · {} not reached |",
        count(Verdict::Pass),
        count(Verdict::Fail),
        count(Verdict::Skip),
        items.len().saturating_sub(answers.len())
    );

    let mut section = None;
    for item in items {
        let Some(answer) = answers.get(&item.id) else {
            continue;
        };
        if section != Some(item.section) {
            section = Some(item.section);
            let _ = writeln!(out);
            let _ = writeln!(out, "## {}. {}", item.section, item.section_title);
            let _ = writeln!(out);
        }
        let _ = writeln!(out, "{}", answer_line(item, answer));
    }
    out
}

/// Read a run file back: its commit and tag, and the answers by item id.
fn parse_run(text: &str) -> (RunHeader, BTreeMap<String, Answer>) {
    let mut header = RunHeader::default();
    let mut answers = BTreeMap::new();
    for line in text.lines() {
        if let Some(started) = line.strip_prefix("# Rig run ") {
            header.started = started.trim().to_string();
        } else if let Some(row) = line.strip_prefix("| ") {
            let cell = |name: &str| {
                row.strip_prefix(name)
                    .and_then(|r| r.strip_prefix(" | "))
                    .map(|r| r.trim_end_matches(" |").trim().to_string())
            };
            if let Some(v) = cell("Commit") {
                header.commit = v;
            } else if let Some(v) = cell("Machine · macOS") {
                header.machine = v;
            } else if let Some(v) = cell("Interface") {
                header.interface = v;
            } else if let Some(v) = cell("Tester") {
                header.tester = v;
            } else if let Some(v) = cell("Items") {
                header.tag = v.trim_matches('`').to_string();
            }
        } else if let Some(rest) = line.strip_prefix("- ") {
            let mut parts = rest.splitn(3, " · ");
            let (Some(word), Some(id), Some(tail)) = (parts.next(), parts.next(), parts.next())
            else {
                continue;
            };
            let Some(verdict) = Verdict::from_word(word) else {
                continue;
            };
            // The item's text is written without asterisks, so the
            // first " — *" can only open the note, whatever dashes the
            // item or the note carry themselves.
            let (body, note) = match tail.split_once(NOTE_OPEN) {
                Some((body, note)) => (body, note.strip_suffix('*').unwrap_or(note).to_string()),
                None => (tail, String::new()),
            };
            let (body, take) = match body.split_once(TAKE_MARK) {
                Some((body, take)) => (body, Some(take.to_string())),
                None => (body, None),
            };
            let stalls = body.split(STALL_MARK).skip(1).map(str::to_string).collect();
            answers.insert(
                id.to_string(),
                Answer {
                    verdict,
                    note,
                    stalls,
                    take,
                },
            );
        }
    }
    (header, answers)
}

/// The commit's short hash: the first word of `git log --oneline`.
fn commit_hash(commit: &str) -> &str {
    commit.split_whitespace().next().unwrap_or("unknown")
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Args {
    tag: String,
    section: Option<u32>,
    checklist: PathBuf,
    out_dir: PathBuf,
    fresh: bool,
    mute: bool,
    /// Print what would be read out and stop.
    list: bool,
    /// Record each item's take.
    record: bool,
    /// The interface to record; the first one plugged in when `None`.
    device: Option<String>,
    takes_dir: PathBuf,
}

fn parse_args(args: &[String]) -> Result<Args> {
    let mut out = Args {
        tag: "hands".to_string(),
        section: None,
        checklist: PathBuf::from(DEFAULT_CHECKLIST),
        out_dir: PathBuf::from(DEFAULT_OUT_DIR),
        fresh: false,
        mute: false,
        list: false,
        record: false,
        device: None,
        takes_dir: PathBuf::from(DEFAULT_TAKES_DIR),
    };
    let mut i = 0;
    while i < args.len() {
        let flag = args[i].as_str();
        let value = || -> Result<&str> {
            args.get(i + 1)
                .map(String::as_str)
                .ok_or_else(|| anyhow!("{flag} expects a value"))
        };
        match flag {
            "--fresh" => {
                out.fresh = true;
                i += 1;
                continue;
            }
            "--mute" => {
                out.mute = true;
                i += 1;
                continue;
            }
            "--list" => {
                out.list = true;
                i += 1;
                continue;
            }
            "--record" => {
                out.record = true;
                i += 1;
                continue;
            }
            "--device" => out.device = Some(value()?.to_string()),
            "--takes" => out.takes_dir = PathBuf::from(value()?),
            "--tag" => out.tag = value()?.to_string(),
            "--section" => out.section = Some(value()?.parse().context("--section not a number")?),
            "--checklist" => out.checklist = PathBuf::from(value()?),
            "--out" => out.out_dir = PathBuf::from(value()?),
            other => bail!("unknown rig-coach argument: {other}\n{USAGE}"),
        }
        i += 2;
    }
    Ok(out)
}

fn command_line(program: &str, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (output.status.success() && !text.is_empty()).then_some(text)
}

fn plugged_interfaces() -> Vec<dub_audio::AudioDevice> {
    dub_audio::list_audio_devices()
        .unwrap_or_default()
        .into_iter()
        .filter(|d| d.category == dub_audio::DeviceCategory::PerformanceInterface)
        .collect()
}

/// The interface a run records and its input count: the one named, else
/// the first DJ interface plugged in. Every input, so a take holds both
/// decks and the aux pair whichever the item uses.
fn recording_device(named: Option<&str>) -> Option<(String, u32)> {
    let device = match named {
        Some(name) => {
            let needle = name.to_lowercase();
            dub_audio::list_audio_devices()
                .unwrap_or_default()
                .into_iter()
                .find(|d| d.input_channels > 0 && d.name.to_lowercase().contains(&needle))
        }
        None => plugged_interfaces().into_iter().next(),
    }?;
    Some((device.name, device.input_channels.max(2)))
}

fn interface_names() -> String {
    let names: Vec<String> = plugged_interfaces().into_iter().map(|d| d.name).collect();
    if names.is_empty() {
        "none plugged in".to_string()
    } else {
        names.join(", ")
    }
}

fn new_header(tag: &str) -> RunHeader {
    let unknown = || "unknown".to_string();
    RunHeader {
        started: command_line("date", &["+%Y-%m-%d %H:%M"]).unwrap_or_else(unknown),
        commit: command_line("git", &["log", "-1", "--format=%h %s"]).unwrap_or_else(unknown),
        machine: format!(
            "{} · {}",
            command_line("sysctl", &["-n", "hw.model"]).unwrap_or_else(unknown),
            command_line("sw_vers", &["-productVersion"]).unwrap_or_else(unknown)
        ),
        interface: interface_names(),
        tester: command_line("git", &["config", "user.name"]).unwrap_or_else(unknown),
        tag: tag.to_string(),
    }
}

fn run_file_name(header: &RunHeader) -> String {
    let stamp: String = header
        .started
        .chars()
        .map(|c| if c == ' ' { '-' } else { c })
        .filter(|c| *c != ':')
        .collect();
    format!("{stamp}-{}-{}.md", commit_hash(&header.commit), header.tag)
}

/// The newest run in `dir` for this commit and tag that still has
/// items of `ids` unanswered.
fn unfinished_run(
    dir: &Path,
    commit: &str,
    tag: &str,
    ids: &BTreeSet<&str>,
) -> Option<(PathBuf, RunHeader, BTreeMap<String, Answer>)> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x == "md"))
        .collect();
    files.sort();
    files.into_iter().rev().find_map(|path| {
        let (header, answers) = parse_run(&std::fs::read_to_string(&path).ok()?);
        let same = commit_hash(&header.commit) == commit_hash(commit) && header.tag == tag;
        let open = ids.iter().any(|id| !answers.contains_key(*id));
        (same && open && !answers.is_empty()).then_some((path, header, answers))
    })
}

/// Where an item's take goes: a folder named as the run file, a file
/// named as the item.
fn take_path(takes_dir: &Path, run_file: &Path, id: &str) -> PathBuf {
    let run = run_file
        .file_stem()
        .map_or_else(|| "run".into(), |s| s.to_string_lossy().into_owned());
    takes_dir.join(run).join(format!("{id}.wav"))
}

/// One item's take: a child process, so a device that fails or hangs
/// never takes the coach down with it. Its output goes to a `.log` beside
/// the take.
struct Take {
    child: Option<Child>,
    path: PathBuf,
}

impl Take {
    /// `dub capture` of the first `channels` inputs of `device` into
    /// `path`.
    fn capture(device: &str, channels: u32, path: &Path) -> Result<Self> {
        let mut command = Command::new(std::env::current_exe().context("finding dub")?);
        command
            .arg("capture")
            .arg(path)
            .args(["--device", device, "--duration", TAKE_CAP_SECS])
            .args(["--channels", &channels.to_string()]);
        Self::spawn(command, path)
    }

    fn spawn(mut command: Command, path: &Path) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
        }
        let log = std::fs::File::create(path.with_extension("log"))
            .with_context(|| format!("creating the log for {}", path.display()))?;
        let child = command
            .stdin(Stdio::piped())
            .stdout(log.try_clone()?)
            .stderr(log)
            .spawn()
            .context("starting the capture")?;
        Ok(Self {
            child: Some(child),
            path: path.to_path_buf(),
        })
    }

    /// A line on stdin ends a capture as Enter does at a terminal; one
    /// that does not finish within `wait` is killed. The take's path when
    /// it holds more than a WAV header.
    fn stop(&mut self, wait: Duration) -> Option<PathBuf> {
        let mut child = self.child.take()?;
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(b"\n");
        }
        let deadline = Instant::now() + wait;
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                _ => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
            }
        }
        let bytes = std::fs::metadata(&self.path).map_or(0, |m| m.len());
        (bytes > 1024).then(|| self.path.clone())
    }
}

impl Drop for Take {
    fn drop(&mut self) {
        let _ = self.stop(TAKE_STOP_WAIT);
    }
}

fn stall_dir() -> Option<PathBuf> {
    Some(dirs_home()?.join("Library/Logs/Dub"))
}

fn dirs_home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn stall_dumps(dir: Option<&Path>) -> BTreeSet<String> {
    let Some(dir) = dir else {
        return BTreeSet::new();
    };
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|name| name.starts_with("stall-"))
                .collect()
        })
        .unwrap_or_default()
}

/// The voice. One utterance at a time: a new one cuts the last off, so
/// a key pressed mid-sentence is answered at once.
struct Voice {
    mute: bool,
    speaking: Option<Child>,
}

impl Voice {
    fn say(&mut self, text: &str) {
        self.hush();
        if self.mute {
            return;
        }
        self.speaking = Command::new("say")
            .args(["-r", SPEECH_RATE, text])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .ok();
    }

    fn hush(&mut self) {
        if let Some(mut child) = self.speaking.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn finish(&mut self) {
        if let Some(mut child) = self.speaking.take() {
            let _ = child.wait();
        }
    }
}

impl Drop for Voice {
    fn drop(&mut self) {
        self.hush();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Key {
    Verdict(Verdict),
    Again,
    Back,
    Quit,
}

fn key_for(code: KeyCode, modifiers: KeyModifiers) -> Option<Key> {
    match code {
        KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => Some(Key::Quit),
        KeyCode::Char(c) => match c.to_ascii_lowercase() {
            'p' => Some(Key::Verdict(Verdict::Pass)),
            'f' => Some(Key::Verdict(Verdict::Fail)),
            's' => Some(Key::Verdict(Verdict::Skip)),
            'r' => Some(Key::Again),
            'b' => Some(Key::Back),
            'q' => Some(Key::Quit),
            _ => None,
        },
        KeyCode::Esc => Some(Key::Quit),
        _ => None,
    }
}

/// Raw mode for as long as this lives, so one key answers without Enter.
struct RawMode;

impl RawMode {
    fn enter() -> Result<Self> {
        terminal::enable_raw_mode().context("the coach needs a terminal to read keys from")?;
        Ok(Self)
    }
}

impl Drop for RawMode {
    fn drop(&mut self) {
        let _ = terminal::disable_raw_mode();
    }
}

fn read_key() -> Result<Key> {
    let _raw = RawMode::enter()?;
    loop {
        if let Event::Key(k) = event::read().context("reading a key")? {
            if k.kind != KeyEventKind::Press {
                continue;
            }
            if let Some(key) = key_for(k.code, k.modifiers) {
                return Ok(key);
            }
        }
    }
}

fn read_note() -> Result<String> {
    print!("  What happened? (one line, Enter) ");
    std::io::stdout().flush().ok();
    let mut line = String::new();
    std::io::stdin()
        .read_line(&mut line)
        .context("reading the note")?;
    Ok(line.trim().to_string())
}

fn show(
    item: &Item,
    position: usize,
    total: usize,
    tag: &str,
    answered: Option<&Answer>,
    recording: Option<&Path>,
) {
    // Clear the screen and home the cursor: one item at a time is the
    // whole point.
    print!("\x1b[2J\x1b[H");
    println!("Rig run · {tag} · {position} of {total}");
    println!();
    println!("{}. {}", item.section, item.section_title);
    println!();
    let text = plain(&item.text);
    let (action, expect) = match text.split_once('→') {
        Some((a, e)) => (a.trim().to_string(), Some(e.trim().to_string())),
        None => (text, None),
    };
    for line in wrap(&action, 68) {
        println!("  {line}");
    }
    if let Some(expect) = expect {
        println!();
        for (n, line) in wrap(&expect, 64).into_iter().enumerate() {
            println!("  {} {line}", if n == 0 { "→" } else { " " });
        }
    }
    println!();
    if let Some(answer) = answered {
        println!("  (answered: {})", answer.verdict.word());
        println!();
    }
    if let Some(take) = recording {
        println!("  ● recording {}", take.display());
        println!();
    }
    println!("  [p] pass   [f] fail   [s] skip   [r] say again   [b] back   [q] quit");
    std::io::stdout().flush().ok();
}

/// `dub rig-coach` — see the module docs.
///
/// # Errors
/// A checklist that can't be read or has no items for the tag, a run
/// file that can't be written, or no terminal to read keys from.
pub fn run(args: &[String]) -> Result<()> {
    let args = parse_args(args)?;
    let markdown = std::fs::read_to_string(&args.checklist).with_context(|| {
        format!(
            "reading {} (run from the repository root)",
            args.checklist.display()
        )
    })?;
    let items: Vec<Item> = parse_checklist(&markdown)
        .into_iter()
        .filter(|i| i.tag == args.tag)
        .filter(|i| args.section.is_none_or(|s| i.section == s))
        .collect();
    if items.is_empty() {
        bail!(
            "no `{}` items{} in {}",
            args.tag,
            args.section
                .map(|s| format!(" in section {s}"))
                .unwrap_or_default(),
            args.checklist.display()
        );
    }

    if args.list {
        for item in &items {
            println!("{:>5}  {}", item.id, spoken(&item.text));
        }
        println!("{} `{}` items", items.len(), args.tag);
        return Ok(());
    }

    let mut header = new_header(&args.tag);
    let ids: BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
    let resumed = if args.fresh {
        None
    } else {
        unfinished_run(&args.out_dir, &header.commit, &args.tag, &ids)
    };
    let (path, mut answers) = match resumed {
        Some((path, old_header, answers)) => {
            header = old_header;
            (path, answers)
        }
        None => (args.out_dir.join(run_file_name(&header)), BTreeMap::new()),
    };
    std::fs::create_dir_all(&args.out_dir)
        .with_context(|| format!("creating {}", args.out_dir.display()))?;
    let save = |answers: &BTreeMap<String, Answer>| -> Result<()> {
        // Every item of the checklist's order, not only this run's
        // filter, so a --section rerun keeps the other sections' answers.
        let all: Vec<Item> = parse_checklist(&markdown)
            .into_iter()
            .filter(|i| i.tag == args.tag)
            .collect();
        std::fs::write(&path, render_run(&header, &all, answers))
            .with_context(|| format!("writing {}", path.display()))
    };

    let mut voice = Voice {
        mute: args.mute,
        speaking: None,
    };
    let stalls_in = stall_dir();
    let mut seen_stalls = stall_dumps(stalls_in.as_deref());

    let recording_from = if args.record {
        let device = recording_device(args.device.as_deref());
        if device.is_none() {
            println!("no interface to record: the items are not recorded");
        }
        device
    } else {
        None
    };
    // The item being recorded and its take. Replacing it stops the old one.
    let mut take: Option<(String, Take)> = None;

    let mut at = items
        .iter()
        .position(|i| !answers.contains_key(&i.id))
        .unwrap_or(0);
    let mut lead = if answers.is_empty() {
        format!("Rig run. {} items.", items.len())
    } else {
        format!("Picking up the run. Item {} of {}.", at + 1, items.len())
    };
    let mut section_said = None;

    while at < items.len() {
        let item = &items[at];
        let mut take_failed = None;
        if let Some((device, channels)) = &recording_from {
            if take.as_ref().is_none_or(|(id, _)| *id != item.id) {
                take = None;
                let to = take_path(&args.takes_dir, &path, &item.id);
                match Take::capture(device, *channels, &to) {
                    Ok(started) => take = Some((item.id.clone(), started)),
                    Err(e) => take_failed = Some(format!("{e:#}")),
                }
            }
        }
        show(
            item,
            at + 1,
            items.len(),
            &args.tag,
            answers.get(&item.id),
            take.as_ref().map(|(_, t)| t.path.as_path()),
        );
        if let Some(why) = take_failed {
            println!("  not recording this item: {why}");
        }
        if section_said != Some(item.section) {
            section_said = Some(item.section);
            let _ = write!(lead, " Section {}: {}.", item.section, item.section_title);
        }
        voice.say(&format!("{lead} {}", spoken(&item.text)));
        lead.clear();

        match read_key()? {
            Key::Again => {}
            Key::Back => {
                at = at.saturating_sub(1);
                section_said = None;
            }
            Key::Quit => {
                voice.hush();
                break;
            }
            Key::Verdict(verdict) => {
                voice.hush();
                // Before the note: the take is the gesture, not the typing.
                let recorded = take.take().and_then(|(_, mut t)| t.stop(TAKE_STOP_WAIT));
                let now = stall_dumps(stalls_in.as_deref());
                let stalls: Vec<String> = now.difference(&seen_stalls).cloned().collect();
                seen_stalls = now;
                let note = if verdict == Verdict::Fail {
                    voice.say("Fail. Type what happened.");
                    println!();
                    read_note()?
                } else {
                    String::new()
                };
                lead = verdict.said().to_string();
                if !stalls.is_empty() {
                    lead.push_str(" A stall dump landed during that item.");
                }
                answers.insert(
                    item.id.clone(),
                    Answer {
                        verdict,
                        note,
                        stalls,
                        take: recorded.map(|p| p.display().to_string()),
                    },
                );
                save(&answers)?;
                at += 1;
            }
        }
    }

    drop(take);
    print!("\x1b[2J\x1b[H");
    let count = |v: Verdict| {
        items
            .iter()
            .filter(|i| answers.get(&i.id).is_some_and(|a| a.verdict == v))
            .count()
    };
    let left = items
        .iter()
        .filter(|i| !answers.contains_key(&i.id))
        .count();
    let summary = format!(
        "{} pass, {} fail, {} skipped{}",
        count(Verdict::Pass),
        count(Verdict::Fail),
        count(Verdict::Skip),
        if left > 0 {
            format!(", {left} to go")
        } else {
            String::new()
        }
    );
    println!("{summary}");
    for item in &items {
        if let Some(answer) = answers.get(&item.id) {
            if answer.verdict == Verdict::Fail || !answer.stalls.is_empty() {
                println!("  {}", answer_line(item, answer));
            }
        }
    }
    if answers.is_empty() {
        println!("nothing answered, nothing written");
    } else {
        println!("written to {}", path.display());
    }
    if left > 0 {
        println!("run it again to pick up where you stopped");
    }
    voice.say(&format!(
        "{lead} {}. {summary}.",
        if left > 0 { "Stopped" } else { "Done" }
    ));
    voice.finish();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHECKLIST: &str = include_str!("../../../docs/RIG-CHECKLIST.md");

    const SMALL: &str = "\
# Rig checklist

- [ ] not in a section, not an item

## Who runs what

- **①** a list that is not a checkbox

## 0. Owed now

- [ ] `hands` **One-turntable swap.** Play a record in THRU,
      ECHO OUT, **HOLD** → the echo carries the swap.
- [ ] `loop ②` **A rip end to end** (section 7).

## 2. Interface and timecode

Some prose between the heading and the items.

- [ ] `hands` Lift the needle mid-play, drop it elsewhere → re-locks, no
      click or runaway.

  A paragraph that is not part of the item above.
- [x] `robot` Already ticked.
- [ ] No tag at all.

## Sign-off

| Section | Robot |
|---|---|
";

    fn item(items: &[Item], id: &str) -> Item {
        items
            .iter()
            .find(|i| i.id == id)
            .cloned()
            .unwrap_or_else(|| panic!("no item {id}"))
    }

    #[test]
    fn parses_items_with_their_section_tag_and_wrapped_text() {
        let items = parse_checklist(SMALL);
        assert_eq!(
            items.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(),
            ["0.1", "0.2", "2.1", "2.2", "2.3"]
        );
        let swap = item(&items, "0.1");
        assert_eq!(swap.tag, "hands");
        assert_eq!(swap.section_title, "Owed now");
        assert_eq!(
            swap.text,
            "**One-turntable swap.** Play a record in THRU, ECHO OUT, **HOLD** → the echo \
             carries the swap."
        );
        assert_eq!(item(&items, "0.2").tag, "loop", "the ② is not the tag");
        assert_eq!(
            item(&items, "2.1").text,
            "Lift the needle mid-play, drop it elsewhere → re-locks, no click or runaway.",
            "a paragraph after a blank line is not the item"
        );
        assert_eq!(item(&items, "2.2").tag, "robot");
        assert_eq!(item(&items, "2.3").tag, "");
    }

    #[test]
    fn every_item_of_the_real_checklist_has_an_owner() {
        let items = parse_checklist(CHECKLIST);
        assert!(items.len() > 50, "only {} items parsed", items.len());
        for i in &items {
            assert!(
                matches!(i.tag.as_str(), "hands" | "robot" | "loop"),
                "item {} has tag {:?}: {}",
                i.id,
                i.tag,
                i.text
            );
            assert!(!i.text.is_empty(), "item {} is empty", i.id);
        }
        assert!(items.iter().any(|i| i.tag == "hands"));
        let ids: BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids.len(), items.len(), "ids are unique");
    }

    #[test]
    fn nothing_in_the_real_checklist_is_read_out_as_a_symbol() {
        for i in parse_checklist(CHECKLIST) {
            let said = spoken(&i.text);
            for sign in ['→', '▸', '*', '`', '−', '″', '♩', '<', '>'] {
                assert!(!said.contains(sign), "item {} says {sign:?}: {said}", i.id);
            }
        }
    }

    #[test]
    fn speaks_signs_as_words() {
        assert_eq!(
            spoken("Pitch −8 → +8 % slowly → **no jump**; Preferences ▸ Alpha"),
            "Pitch minus 8. Expect: +8 % slowly. Expect: no jump; Preferences, Alpha"
        );
        assert_eq!(
            spoken("a dnb 12″ at `♩ A · <bpm>`"),
            "a dnb 12 inch at A, bpm"
        );
    }

    #[test]
    fn wraps_on_words() {
        assert_eq!(wrap("one two three four", 9), ["one two", "three", "four"]);
        assert_eq!(wrap("", 9), Vec::<String>::new());
        assert_eq!(wrap("unbreakable", 4), ["unbreakable"]);
    }

    fn header() -> RunHeader {
        RunHeader {
            started: "2026-11-27 14:05".into(),
            commit: "9d88a4b feat: calibration names the record".into(),
            machine: "MacBookPro18,3 · 14.6.1".into(),
            interface: "SL 3".into(),
            tester: "A DJ".into(),
            tag: "hands".into(),
        }
    }

    #[test]
    fn a_run_file_reads_back_as_it_was_written() {
        let items: Vec<Item> = parse_checklist(SMALL)
            .into_iter()
            .filter(|i| i.tag == "hands")
            .collect();
        let mut answers = BTreeMap::new();
        answers.insert(
            "0.1".to_string(),
            Answer {
                verdict: Verdict::Fail,
                note: "the echo grew — twice".into(),
                stalls: vec!["stall-1.txt".into(), "stall-2.txt".into()],
                take: Some("testdata/rig-takes/2026-11-27-1405-9d88a4b-hands/0.1.wav".into()),
            },
        );
        answers.insert(
            "2.1".to_string(),
            Answer {
                verdict: Verdict::Pass,
                note: String::new(),
                stalls: vec![],
                take: None,
            },
        );
        let text = render_run(&header(), &items, &answers);
        assert!(text.contains("| Result | 1 pass · 1 fail · 0 skipped · 0 not reached |"));
        assert!(text.contains("## 0. Owed now"));
        assert!(text.contains("- PASS · 2.1 · Lift the needle"));

        let (read_header, read_answers) = parse_run(&text);
        assert_eq!(read_header, header());
        assert_eq!(read_answers, answers);
    }

    #[test]
    fn an_unanswered_item_is_counted_and_left_out() {
        let items: Vec<Item> = parse_checklist(SMALL)
            .into_iter()
            .filter(|i| i.tag == "hands")
            .collect();
        let text = render_run(&header(), &items, &BTreeMap::new());
        assert!(text.contains("2 not reached"));
        assert!(!text.contains("## 0."));
        assert!(parse_run(&text).1.is_empty());
    }

    #[test]
    fn the_file_name_sorts_by_time_and_names_the_commit() {
        assert_eq!(run_file_name(&header()), "2026-11-27-1405-9d88a4b-hands.md");
    }

    #[test]
    fn picks_up_the_newest_unfinished_run_of_this_commit() {
        let dir = std::env::temp_dir().join(format!("dub-rig-coach-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let items: Vec<Item> = parse_checklist(SMALL)
            .into_iter()
            .filter(|i| i.tag == "hands")
            .collect();
        let ids: BTreeSet<&str> = items.iter().map(|i| i.id.as_str()).collect();
        let one = |id: &str| {
            let mut a = BTreeMap::new();
            a.insert(
                id.to_string(),
                Answer {
                    verdict: Verdict::Pass,
                    note: String::new(),
                    stalls: vec![],
                    take: None,
                },
            );
            a
        };
        let write = |name: &str, h: &RunHeader, a: &BTreeMap<String, Answer>| {
            std::fs::write(dir.join(name), render_run(h, &items, a)).unwrap();
        };

        let mut other_commit = header();
        other_commit.commit = "1111111 other".into();
        let mut finished = one("0.1");
        finished.extend(one("2.1"));
        write("2026-11-27-1000-9d88a4b-hands.md", &header(), &one("0.1"));
        write("2026-11-27-1100-9d88a4b-hands.md", &header(), &finished);
        write(
            "2026-11-27-1200-1111111-hands.md",
            &other_commit,
            &one("0.1"),
        );

        let (path, _, answers) =
            unfinished_run(&dir, &header().commit, "hands", &ids).expect("the 10:00 run is open");
        assert!(path.ends_with("2026-11-27-1000-9d88a4b-hands.md"));
        assert_eq!(answers.len(), 1);
        assert!(unfinished_run(&dir, &header().commit, "loop", &ids).is_none());

        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn keys() {
        let none = KeyModifiers::NONE;
        assert_eq!(
            key_for(KeyCode::Char('p'), none),
            Some(Key::Verdict(Verdict::Pass))
        );
        assert_eq!(
            key_for(KeyCode::Char('F'), KeyModifiers::SHIFT),
            Some(Key::Verdict(Verdict::Fail))
        );
        assert_eq!(key_for(KeyCode::Char('r'), none), Some(Key::Again));
        assert_eq!(key_for(KeyCode::Char('b'), none), Some(Key::Back));
        assert_eq!(
            key_for(KeyCode::Char('c'), KeyModifiers::CONTROL),
            Some(Key::Quit)
        );
        assert_eq!(key_for(KeyCode::Enter, none), None, "Enter answers nothing");
        assert_eq!(key_for(KeyCode::Char(' '), none), None);
    }

    #[test]
    fn parses_arguments() {
        let s = |a: &[&str]| a.iter().map(|x| (*x).to_string()).collect::<Vec<_>>();
        let a = parse_args(&s(&[])).unwrap();
        assert_eq!(a.tag, "hands");
        assert_eq!(a.out_dir, PathBuf::from("docs/rig-runs"));
        let a = parse_args(&s(&[
            "--section",
            "2",
            "--mute",
            "--fresh",
            "--tag",
            "loop",
        ]))
        .unwrap();
        assert_eq!((a.section, a.mute, a.fresh), (Some(2), true, true));
        assert_eq!(a.tag, "loop");
        assert!(parse_args(&s(&["--list"])).unwrap().list);
        assert!(parse_args(&s(&["--section"])).is_err());
        assert!(parse_args(&s(&["--loud"])).is_err());
        assert!(!a.record);
        let a = parse_args(&s(&["--record", "--device", "SL 3", "--takes", "/tmp/t"])).unwrap();
        assert!(a.record);
        assert_eq!(a.device.as_deref(), Some("SL 3"));
        assert_eq!(a.takes_dir, PathBuf::from("/tmp/t"));
        assert_eq!(
            parse_args(&s(&[])).unwrap().takes_dir,
            PathBuf::from("testdata/rig-takes")
        );
    }

    /// One folder per run, named as the run file, one take per item.
    #[test]
    fn a_take_is_named_by_its_run_and_its_item() {
        let run = Path::new("docs/rig-runs/2026-11-28-1500-abc1234-hands.md");
        assert_eq!(
            take_path(Path::new("testdata/rig-takes"), run, "2.3"),
            PathBuf::from("testdata/rig-takes/2026-11-28-1500-abc1234-hands/2.3.wav")
        );
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("dub-rig-take-{name}-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join("2.3.wav")
    }

    /// The answer ends the take as Enter ends `dub capture`: a line on
    /// its stdin, and the file it wrote is the take.
    #[test]
    fn a_line_on_stdin_ends_the_take_and_keeps_its_file() {
        let path = scratch("ends");
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "read line; head -c 4096 /dev/zero > \"$0\""])
            .arg(&path);
        let mut take = Take::spawn(command, &path).unwrap();
        assert_eq!(take.stop(Duration::from_secs(5)), Some(path.clone()));
    }

    /// A capture that does not answer is stopped, not waited on, and a
    /// take with no audio in it is not named in the run.
    #[test]
    fn a_capture_that_hangs_is_killed_and_left_out() {
        let path = scratch("hangs");
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "exec sleep 30"]);
        let mut take = Take::spawn(command, &path).unwrap();
        let started = Instant::now();
        assert_eq!(take.stop(Duration::from_millis(300)), None);
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

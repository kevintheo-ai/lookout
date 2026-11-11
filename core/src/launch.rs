//! New sessions from the inbox's + (Kevin 2026-11-10): the places and programs to choose from come from the settings
//! (`newSession`), the three favorites from the app's memory. This module checks them and turns a favorite and a task
//! into a start script for `/bin/sh`. The terminal only runs `/bin/sh <script>`, so no text of the user ever meets the
//! user's own shell: fish reads quotes differently from sh (plan review 2026-11-10).

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::config::standardize_path;
use crate::model::trim_ws;

/// The word of a program's command that stands for the task. The word before it must be `--`: a task that starts with
/// a dash is then never read as an option (`--version` would print the version and close the window).
pub const TASK: &str = "{task}";

/// The word of a program's command that stands for the session's name (a team's windows, Kevin 2026-11-10): a word of
/// its own, right after its option (`--name {name}`), at most once (`command_ok`). Without a name both words go.
pub const NAME: &str = "{name}";

/// Favorites at the +.
pub const FAVORITES: usize = 3;

/// Where a session starts: a folder, and values the command can use as `{key}`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    pub folder: String,
    pub values: BTreeMap<String, String>,
}

/// A program: the models and efforts to choose from, and its command as a word list. `models_command` (optional, a
/// word list too) gives the current models once a day, one per line (Kevin 2026-11-10), each optionally followed by a
/// tab and the name the menu shows (`opus<TAB>Opus 5.5`: the command gets `opus`, the menu shows the number);
/// `models` is the list until it answered and whenever it fails.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Program {
    pub name: String,
    pub models: Vec<String>,
    pub efforts: Vec<String>,
    pub command: Vec<String>,
    pub models_command: Vec<String>,
    pub model_labels: BTreeMap<String, String>,
}

impl Program {
    /// Whether it can give the session a name (`{name}` in its command): only such programs can be a team's lead or key
    /// account, Lookout knows the team by the name.
    pub fn takes_name(&self) -> bool {
        self.command.iter().any(|w| w == NAME)
    }
}

/// At most this many models from a `modelsCommand`.
pub const MODELS_MAX: usize = 40;

/// What `newSession` offers; never empty, the defaults fill in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewSession {
    pub places: Vec<Place>,
    pub programs: Vec<Program>,
    /// The role texts of a new team (`newSession.team`, `team_start`).
    pub team: crate::team_start::TeamTexts,
}

/// A favorite: names into the lists of `NewSession`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Favorite {
    pub place: String,
    pub program: String,
    pub model: String,
    pub effort: String,
}

/// A part of a favorite, as the menu changes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Place,
    Program,
    Model,
    Effort,
}

impl Field {
    pub const ALL: [Field; 4] = [Field::Place, Field::Program, Field::Model, Field::Effort];

    /// Its key in a menu item.
    pub fn key(self) -> &'static str {
        match self {
            Field::Place => "place",
            Field::Program => "program",
            Field::Model => "model",
            Field::Effort => "effort",
        }
    }

    pub fn from_key(key: &str) -> Option<Field> {
        Field::ALL.into_iter().find(|f| f.key() == key)
    }
}

impl NewSession {
    /// Without settings: the home folder and Claude Code.
    pub fn defaults(home: &str) -> NewSession {
        NewSession {
            places: vec![Place {
                name: "Home".into(),
                folder: home.to_owned(),
                values: BTreeMap::new(),
            }],
            programs: vec![Program {
                name: "Claude Code".into(),
                models: words(&["opus", "sonnet", "haiku"]),
                efforts: words(&["max", "xhigh", "high", "medium", "low"]),
                command: words(&[
                    "claude", "--name", NAME, "--model", "{model}", "--effort", "{effort}", "--",
                    TASK,
                ]),
                models_command: vec![],
                model_labels: BTreeMap::new(),
            }],
            team: crate::team_start::TeamTexts::defaults(),
        }
    }

    /// The programs that ask for their models, with the command.
    pub fn models_commands(&self) -> Vec<(String, Vec<String>)> {
        self.programs
            .iter()
            .filter(|p| !p.models_command.is_empty())
            .map(|p| (p.name.clone(), p.models_command.clone()))
            .collect()
    }

    /// The models a program answered with (`model_lines`: a model, optionally a tab and its name in the menu); an empty
    /// answer keeps what was there. A name that another row would show too gets its model in brackets, so two rows never
    /// look the same (review quick 2026-11-10).
    pub fn set_models(&mut self, program: &str, lines: &[String]) {
        if let Some(p) = self.programs.iter_mut().find(|p| p.name == program)
            && !lines.is_empty()
        {
            let split = |l: &String| {
                l.split_once('\t').map_or((l.clone(), None), |(m, n)| {
                    (m.to_owned(), Some(n.to_owned()))
                })
            };
            p.models = lines.iter().map(|l| split(l).0).collect();
            let mut named = BTreeSet::new();
            p.model_labels = lines
                .iter()
                .filter_map(|l| match split(l) {
                    (m, Some(n)) => {
                        let clash = (n != m && p.models.contains(&n)) || !named.insert(n.clone());
                        let shown = if clash { format!("{n} ({m})") } else { n };
                        Some((m, shown))
                    }
                    _ => None,
                })
                .collect();
        }
    }

    /// What the menu shows for `model` of `program`: its name from the model list, else the model itself.
    pub fn model_label<'a>(&'a self, program: &str, model: &'a str) -> &'a str {
        self.program(program)
            .and_then(|p| p.model_labels.get(model))
            .map_or(model, String::as_str)
    }

    /// "Place · Program · model · effort" with the model's name from the list (`Favorite::label` otherwise).
    pub fn label(&self, f: &Favorite) -> String {
        Favorite {
            model: self.model_label(&f.program, &f.model).to_owned(),
            ..f.clone()
        }
        .label()
    }

    pub fn place(&self, name: &str) -> Option<&Place> {
        self.places.iter().find(|p| p.name == name)
    }

    pub fn program(&self, name: &str) -> Option<&Program> {
        self.programs.iter().find(|p| p.name == name)
    }

    /// The place and program of `f`, if both exist and the program offers its model and effort.
    pub fn resolve(&self, f: &Favorite) -> Option<(&Place, &Program)> {
        let place = self.place(&f.place)?;
        let program = self.program(&f.program)?;
        (offers(&program.models, &f.model) && offers(&program.efforts, &f.effort))
            .then_some((place, program))
    }

    /// The three favorites: those the app stored (`favorites_to_json`), then the first choice of everything for a slot
    /// left empty. A stored favorite that no longer resolves stays as it is: the menu shows it as not available, so a
    /// place renamed in the settings never silently starts somewhere else.
    pub fn favorites(&self, stored: Option<&str>) -> Vec<Favorite> {
        let parsed = stored.and_then(|s| serde_json::from_str::<Value>(s).ok());
        let mut out: Vec<Favorite> = parsed
            .as_ref()
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_object)
            .map(|o| Favorite {
                place: field(o, "place"),
                program: field(o, "program"),
                model: field(o, "model"),
                effort: field(o, "effort"),
            })
            .take(FAVORITES)
            .collect();
        while out.len() < FAVORITES {
            out.push(self.first());
        }
        out
    }

    fn first(&self) -> Favorite {
        let program = &self.programs[0];
        Favorite {
            place: self.places[0].name.clone(),
            program: program.name.clone(),
            model: program.models.first().cloned().unwrap_or_default(),
            effort: program.efforts.first().cloned().unwrap_or_default(),
        }
    }

    /// What `field` of `f` can be: the places, the programs, or the models or efforts of its program (none while that
    /// program is gone from the settings).
    pub fn options(&self, f: &Favorite, field: Field) -> Vec<&str> {
        let program = self.program(&f.program);
        match field {
            Field::Place => self.places.iter().map(|p| p.name.as_str()).collect(),
            Field::Program => self.programs.iter().map(|p| p.name.as_str()).collect(),
            Field::Model => {
                program.map_or(vec![], |p| p.models.iter().map(String::as_str).collect())
            }
            Field::Effort => {
                program.map_or(vec![], |p| p.efforts.iter().map(String::as_str).collect())
            }
        }
    }

    /// `f` with `field` set to `value`. Another program keeps model and effort only if it offers them
    /// (`Favorite::with_program`); a program that is not in the settings changes nothing.
    pub fn change(&self, f: &Favorite, field: Field, value: &str) -> Favorite {
        let mut out = f.clone();
        match field {
            Field::Place => out.place = value.to_owned(),
            Field::Program => {
                if let Some(p) = self.program(value) {
                    out = f.with_program(p);
                }
            }
            Field::Model => out.model = value.to_owned(),
            Field::Effort => out.effort = value.to_owned(),
        }
        out
    }
}

impl Favorite {
    pub fn get(&self, field: Field) -> &str {
        match field {
            Field::Place => &self.place,
            Field::Program => &self.program,
            Field::Model => &self.model,
            Field::Effort => &self.effort,
        }
    }

    /// "Place · Program · model · effort", empty parts left out.
    pub fn label(&self) -> String {
        [&self.place, &self.program, &self.model, &self.effort]
            .into_iter()
            .filter(|s| !s.is_empty())
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(" · ")
    }

    /// Another program: model and effort stay if it offers them, else its first ones.
    pub fn with_program(&self, p: &Program) -> Favorite {
        let keep = |list: &[String], old: &str| {
            if offers(list, old) {
                old.to_owned()
            } else {
                list.first().cloned().unwrap_or_default()
            }
        };
        Favorite {
            place: self.place.clone(),
            program: p.name.clone(),
            model: keep(&p.models, &self.model),
            effort: keep(&p.efforts, &self.effort),
        }
    }
}

/// The favorites as the app stores them.
pub fn favorites_to_json(favorites: &[Favorite]) -> String {
    Value::Array(favorites.iter().map(favorite_value).collect()).to_string()
}

/// One favorite as text, for a menu item ("Choose freely", Kevin 2026-11-10); `favorite_from_key` reads it back.
pub fn favorite_key(f: &Favorite) -> String {
    favorite_value(f).to_string()
}

pub fn favorite_from_key(key: &str) -> Option<Favorite> {
    let v: Value = serde_json::from_str(key).ok()?;
    let o = v.as_object()?;
    Some(Favorite {
        place: field(o, "place"),
        program: field(o, "program"),
        model: field(o, "model"),
        effort: field(o, "effort"),
    })
}

fn favorite_value(f: &Favorite) -> Value {
    json!({"place": f.place, "program": f.program, "model": f.model, "effort": f.effort})
}

/// `newSession` of the settings: `places` (`name`, `folder` and any other string values for the command) and `programs`
/// (`name`, `models`, `efforts`, `command` as a word list, each word kept exactly as written, and optionally
/// `modelsCommand`, a word list too, see `Program`). Dropped: an entry whose
/// name came before (the first one counts, so a favorite never starts in the wrong folder or with the wrong command), a
/// place whose folder is not absolute (after `~`), and a program whose command lacks `--` right before a single
/// `{task}` word or uses `{model}` or `{effort}` without a list. Nothing usable left: the defaults. `{model}`,
/// `{effort}`, `{task}` and `{name}` always come from the favorite and the dialog, a place value of that name never
/// counts. `team` holds the role texts of a new team (`team_start::TeamTexts`).
pub fn parse(v: Option<&Value>, home: &str) -> NewSession {
    let defaults = NewSession::defaults(home);
    let Some(obj) = v.and_then(Value::as_object) else {
        return defaults;
    };
    let mut seen = BTreeSet::new();
    let places: Vec<Place> = items(obj, "places")
        .filter_map(|p| {
            let values = p
                .iter()
                .filter(|(k, _)| *k != "name" && *k != "folder")
                .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                .collect();
            Some(Place {
                name: text(p, "name")?,
                folder: standardize_path(&text(p, "folder")?, home),
                values,
            })
        })
        .filter(|p| p.folder.starts_with('/'))
        .filter(|p| seen.insert(p.name.clone()))
        .collect();
    seen.clear();
    let programs: Vec<Program> = items(obj, "programs")
        .filter_map(|p| {
            let program = Program {
                name: text(p, "name")?,
                models: list(p, "models"),
                efforts: list(p, "efforts"),
                command: exact_words(p, "command"),
                models_command: exact_words(p, "modelsCommand"),
                model_labels: BTreeMap::new(),
            };
            command_ok(&program).then_some(program)
        })
        .filter(|p| seen.insert(p.name.clone()))
        .collect();
    NewSession {
        places: if places.is_empty() {
            defaults.places
        } else {
            places
        },
        programs: if programs.is_empty() {
            defaults.programs
        } else {
            programs
        },
        team: crate::team_start::TeamTexts::parse(obj.get("team")),
    }
}

fn command_ok(p: &Program) -> bool {
    let c = &p.command;
    let uses = |token: &str| c.iter().any(|w| w.contains(token));
    c.iter().filter(|w| w.contains(TASK)).count() == 1
        && c.iter()
            .position(|w| w == TASK)
            .is_some_and(|i| i > 0 && c[i - 1] == "--")
        && (!uses("{model}") || !p.models.is_empty())
        && (!uses("{effort}") || !p.efforts.is_empty())
        && name_ok(c)
}

/// `{name}` only as a word of its own right after an option, at most once (review quick 2026-11-10): without a name the
/// two words go together, so neither a dangling option (`--session team-{name}`) nor the program before it
/// (`tool {name}`) can stay or go wrongly.
fn name_ok(c: &[String]) -> bool {
    let at: Vec<usize> = (0..c.len()).filter(|&i| c[i].contains(NAME)).collect();
    match at[..] {
        [] => true,
        [i] => c[i] == NAME && i > 1 && c[i - 1].starts_with('-') && c[i - 1] != "--",
        _ => false,
    }
}

/// The start script for `/bin/sh`. It removes itself first (the task need not stay on disk), goes into the place's
/// folder, then turns into the program's command (`exec`: no shell stays between the terminal's shell and the session)
/// with model, effort and the place's values put in, in one pass. Every word is quoted as a whole afterwards, so nothing
/// in a value or in the task is ever read by the shell; the task becomes one line. An empty task leaves out `--` and
/// `{task}`: the session starts without one.
pub fn start_script(place: &Place, program: &Program, f: &Favorite, task: &str) -> String {
    start_script_named(place, program, f, task, "")
}

/// `start_script` with the session's name for `{name}`; an empty name leaves out `{name}` and its option.
pub fn start_script_named(
    place: &Place,
    program: &Program,
    f: &Favorite,
    task: &str,
    name: &str,
) -> String {
    let task = task.split_whitespace().collect::<Vec<_>>().join(" ");
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut vars: BTreeMap<&str, &str> = place
        .values
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    vars.insert("model", &f.model);
    vars.insert("effort", &f.effort);
    vars.insert("name", &name);
    let c = &program.command;
    let before = |i: usize, word: &str| c.get(i + 1).is_some_and(|next| next == word);
    let command: Vec<String> = c
        .iter()
        .enumerate()
        .filter(|(i, w)| {
            (!task.is_empty() || (*w != TASK && !before(*i, TASK)))
                && (!name.is_empty() || (*w != NAME && !before(*i, NAME)))
        })
        .map(|(_, w)| {
            if w == TASK {
                sh_quote(&task)
            } else {
                sh_quote(&fill(w, &vars))
            }
        })
        .collect();
    // A name also goes to the environment as `CLAUDE_SESSION_NAME`: a hook that names sessions by their first message
    // keeps it then instead of renaming the window, so the team keeps its lead (live 2026-11-10: "Test · Team-Lead"
    // became "Test · Arbeit in Stufen planen").
    let env = if name.is_empty() {
        String::new()
    } else {
        format!("export CLAUDE_SESSION_NAME={}\n", sh_quote(&name))
    };
    format!(
        "rm -f -- \"$0\"\ncd {} || exit 1\n{env}exec {}\n",
        sh_quote(&place.folder),
        command.join(" ")
    )
}

/// What the terminal runs: `/bin/sh <script>`. Only for a path of plain characters, which every shell reads the same
/// way without quotes (fish, zsh, bash); anything else gives `None`.
pub fn launch_exec(script: &str) -> Option<String> {
    script
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || "/._-".contains(ch))
        .then(|| format!("/bin/sh {script}"))
}

/// `{name}` in `word` replaced from `vars` in one pass: a value that itself holds `{…}` is not expanded again. Unknown
/// names stay as they are.
pub(crate) fn fill(word: &str, vars: &BTreeMap<&str, &str>) -> String {
    let mut out = String::new();
    let mut rest = word;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let tail = &rest[open..];
        match tail.find('}').map(|close| (&tail[1..close], close)) {
            Some((name, close)) if vars.contains_key(name) => {
                out.push_str(vars[name]);
                rest = &tail[close + 1..];
            }
            _ => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// One word for sh: in single quotes, a quote inside as `'\''`.
fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn offers(list: &[String], value: &str) -> bool {
    if list.is_empty() {
        value.is_empty()
    } else {
        list.iter().any(|x| x == value)
    }
}

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| (*s).to_owned()).collect()
}

fn items<'a>(
    obj: &'a Map<String, Value>,
    key: &str,
) -> impl Iterator<Item = &'a Map<String, Value>> {
    obj.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_object)
}

fn text(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(trim_ws)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn list(obj: &Map<String, Value>, key: &str) -> Vec<String> {
    obj.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(trim_ws)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// A word list kept exactly as written, empty words too.
fn exact_words(obj: &Map<String, Value>, key: &str) -> Vec<String> {
    obj.get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect()
}

/// A `modelsCommand`'s answer: one model per line, optionally a tab and the name the menu shows; blank lines and repeated
/// models dropped, at most `MODELS_MAX`. Each line comes back as "model" or "model<TAB>name".
pub fn model_lines(output: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut seen = BTreeSet::new();
    for line in output.lines() {
        let (model, name) = match line.split_once('\t') {
            Some((m, n)) => (trim_ws(m), trim_ws(n)),
            None => (trim_ws(line), ""),
        };
        if model.is_empty() || out.len() >= MODELS_MAX || !seen.insert(model.to_owned()) {
            continue;
        }
        out.push(if name.is_empty() {
            model.to_owned()
        } else {
            format!("{model}\t{name}")
        });
    }
    out
}

/// The asked model lists as kept on disk: the day they were asked, and per program its list.
pub fn models_cache_to_json(day: &str, lists: &BTreeMap<String, Vec<String>>) -> String {
    json!({"day": day, "programs": lists}).to_string()
}

pub fn models_cache_from_json(text: &str) -> Option<(String, BTreeMap<String, Vec<String>>)> {
    let v: Value = serde_json::from_str(text).ok()?;
    let o = v.as_object()?;
    let lists = o
        .get("programs")?
        .as_object()?
        .iter()
        .map(|(k, v)| {
            let list = v.as_array().into_iter().flatten().filter_map(Value::as_str);
            (k.clone(), model_lines(&list.collect::<Vec<_>>().join("\n")))
        })
        .collect();
    Some((o.get("day")?.as_str()?.to_owned(), lists))
}

fn field(obj: &Map<String, Value>, key: &str) -> String {
    obj.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    const HOME: &str = "/home/test";
    /// Prints its arguments joined by `|`, ending in `|`, with the shell's own printf. An outside `printf` is no good:
    /// GNU's (Linux CI) drops a `--` argument, macOS's keeps it.
    const ECHO: &str = "IFS='|'; printf '%s|' \"$*\"";

    fn settings() -> NewSession {
        parse(
            Some(&json!({
                "places": [
                    {"name": "Acme", "folder": "~/acme", "context": "acme"},
                    {"name": "Studio", "folder": "/work/studio", "context": "studio"},
                    {"name": "Acme", "folder": "/elsewhere"},
                    {"name": "No folder"},
                    {"name": "Relative", "folder": "work/repo"},
                    {"name": "Tilde word", "folder": "~repo"}
                ],
                "programs": [
                    {"name": "Tool", "models": ["big", "k3[1m]"], "efforts": ["max", "low"],
                     "command": ["launch", "--model", "{model}", "-c", "effort={effort}", "{context}", "tool", "--", "{task}"],
                     "modelsCommand": ["/bin/echo", ""]},
                    {"name": "Tool", "models": ["other"], "efforts": ["max"], "command": ["x", "--", "{task}"]},
                    {"name": "No dashes", "models": ["m"], "efforts": ["e"], "command": ["x", "{task}"]},
                    {"name": "Quoted task", "models": ["m"], "efforts": ["e"], "command": ["x", "--", "\"{task}\""]},
                    {"name": "No models", "efforts": ["e"], "command": ["x", "--model", "{model}", "--", "{task}"]},
                    {"name": "Plain", "command": ["plain", "--", "{task}"]},
                    {"name": "Wrapper", "command": ["/bin/sh", "-c", ECHO, "", "--", "{task}"]},
                    {"name": "Name glued", "command": ["deploy", "--session", "team-{name}", "--", "{task}"]},
                    {"name": "Name bare", "command": ["tool", "{name}", "--", "{task}"]},
                    {"name": "Name twice", "command": ["t", "-n", "{name}", "-m", "{name}", "--", "{task}"]}
                ]
            })),
            HOME,
        )
    }

    /// Duplicate names, relative folders (they would start in whatever folder the terminal is in) and broken commands
    /// are dropped, `{name}` other than a word of its own after an option too; the first entry of a name counts, `~` is
    /// expanded.
    #[test]
    fn settings_drop_what_could_start_the_wrong_thing() {
        let s = settings();
        let places: Vec<(&str, &str)> = s
            .places
            .iter()
            .map(|p| (p.name.as_str(), p.folder.as_str()))
            .collect();
        assert_eq!(
            places,
            [("Acme", "/home/test/acme"), ("Studio", "/work/studio")]
        );
        let programs: Vec<&str> = s.programs.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(programs, ["Tool", "Plain", "Wrapper"]);
        assert_eq!(s.programs[0].models, ["big", "k3[1m]"], "the first Tool");
        assert_eq!(s.place("Acme").unwrap().values["context"], "acme");
    }

    /// A command's words stay exactly as written, an empty one too (review quick 2026-11-10): `sh -c '…' '' -- {task}`
    /// needs its empty `$0`, without it `--` became `$0` and a task like `--version` reached the program as an option.
    #[test]
    fn empty_words_keep_their_place() {
        let s = settings();
        let wrapper = s.program("Wrapper").unwrap();
        assert_eq!(wrapper.command[3], "");
        let dir = std::env::temp_dir().join(format!("lookout-wrapper-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let place = Place {
            name: "Here".into(),
            folder: dir.to_string_lossy().into_owned(),
            values: BTreeMap::new(),
        };
        let f = Favorite {
            place: "Here".into(),
            program: "Wrapper".into(),
            ..Favorite::default()
        };
        let path = dir.join("start.sh");
        std::fs::write(&path, start_script(&place, wrapper, &f, "--version")).unwrap();
        let out = std::process::Command::new("/bin/sh")
            .arg(&path)
            .output()
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "--|--version|");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn no_settings_give_the_defaults() {
        assert_eq!(parse(None, HOME), NewSession::defaults(HOME));
        assert_eq!(
            parse(
                Some(&json!({"places": [], "programs": [{"name": "x"}]})),
                HOME
            ),
            NewSession::defaults(HOME)
        );
    }

    /// Stored favorites come back as they were; missing slots get the first choice; one that no longer resolves stays
    /// (shown as not available) instead of turning into another.
    #[test]
    fn favorites_round_trip_and_fill_up() {
        let s = settings();
        let one = Favorite {
            place: "Studio".into(),
            program: "Tool".into(),
            model: "k3[1m]".into(),
            effort: "low".into(),
        };
        let gone = Favorite {
            place: "Renamed".into(),
            ..one.clone()
        };
        let stored = favorites_to_json(&[one.clone(), gone.clone()]);
        let favorites = s.favorites(Some(&stored));
        assert_eq!(favorites.len(), FAVORITES);
        assert_eq!(favorites[0], one);
        assert_eq!(favorites[1], gone);
        assert_eq!(favorites[2].label(), "Acme · Tool · big · max");
        assert!(s.resolve(&favorites[0]).is_some());
        assert!(s.resolve(&gone).is_none());
        assert_eq!(s.favorites(Some("kaputt")).len(), FAVORITES);
        let odd = Favorite {
            place: "A \"b\"\nc".into(),
            ..one.clone()
        };
        assert_eq!(favorite_from_key(&favorite_key(&odd)), Some(odd));
        assert_eq!(favorite_from_key("kaputt"), None);
    }

    #[test]
    fn another_program_keeps_what_it_offers() {
        let s = settings();
        let f = Favorite {
            place: "Acme".into(),
            program: "Tool".into(),
            model: "big".into(),
            effort: "low".into(),
        };
        let plain = f.with_program(s.program("Plain").unwrap());
        assert_eq!((plain.model.as_str(), plain.effort.as_str()), ("", ""));
        assert!(s.resolve(&plain).is_some());
        assert_eq!(plain.with_program(s.program("Tool").unwrap()).model, "big");
    }

    /// The daily model lists (Kevin 2026-11-10): a `modelsCommand` is read word for word; its answer is one model per
    /// line; an empty answer keeps the list; the lists survive the disk with their day.
    #[test]
    fn model_lists_come_once_a_day() {
        let mut s = settings();
        assert_eq!(
            s.models_commands(),
            [("Tool".to_owned(), words(&["/bin/echo", ""]))]
        );
        let answer = model_lines(" grok-4.7 \n\n grok-4.6\ngrok-4.7\r\n");
        assert_eq!(answer, ["grok-4.7", "grok-4.6"]);
        assert_eq!(model_lines(&"m\n".repeat(3)).len(), 1, "repeats count once");
        let many: String = (0..100).map(|k| format!("m{k}\n")).collect();
        assert_eq!(model_lines(&many).len(), MODELS_MAX);
        s.set_models("Tool", &answer);
        assert_eq!(s.program("Tool").unwrap().models, answer);
        s.set_models("Tool", &[]);
        assert_eq!(s.program("Tool").unwrap().models, answer, "empty keeps");
        s.set_models("Unknown", &answer);
        // A name for the menu after a tab (Kevin 2026-11-10: "bei claude sehe ich die modell namen mit nummer immer
        // noch nicht"): the command keeps the model, the menu and the favorite's text show the name.
        let named = model_lines("big\tBig 5.5\nsmall\t \nbig\tagain\n\tnameless\n");
        assert_eq!(named, ["big\tBig 5.5", "small"]);
        s.set_models("Tool", &named);
        assert_eq!(s.program("Tool").unwrap().models, ["big", "small"]);
        assert_eq!(s.model_label("Tool", "big"), "Big 5.5");
        assert_eq!(s.model_label("Tool", "small"), "small");
        let f = Favorite {
            place: "Acme".into(),
            program: "Tool".into(),
            model: "big".into(),
            effort: "max".into(),
        };
        assert_eq!(s.label(&f), "Acme · Tool · Big 5.5 · max");
        assert!(
            s.resolve(&f).is_some(),
            "the favorite still names the model"
        );
        // Two rows never look the same: a repeated name or one that is another model gets the model in brackets.
        s.set_models("Tool", &model_lines("a\tX\nb\tX\nc\ta\n"));
        let shown: Vec<&str> = ["a", "b", "c"]
            .iter()
            .map(|m| s.model_label("Tool", m))
            .collect();
        assert_eq!(shown, ["X", "X (b)", "a (c)"]);
        let lists = BTreeMap::from([("Tool".to_owned(), answer.clone())]);
        let text = models_cache_to_json("2026-11-10", &lists);
        assert_eq!(
            models_cache_from_json(&text),
            Some(("2026-11-10".to_owned(), lists))
        );
        assert_eq!(models_cache_from_json("kaputt"), None);
    }

    /// The menu's choices: every place and program, the models and efforts of the favorite's program, none while that
    /// program is gone. A change sets one part; choosing what is set changes nothing.
    #[test]
    fn the_menu_changes_one_part() {
        let s = settings();
        let f = Favorite {
            place: "Acme".into(),
            program: "Tool".into(),
            model: "k3[1m]".into(),
            effort: "low".into(),
        };
        assert_eq!(s.options(&f, Field::Place), ["Acme", "Studio"]);
        assert_eq!(s.options(&f, Field::Program), ["Tool", "Plain", "Wrapper"]);
        assert_eq!(s.options(&f, Field::Model), ["big", "k3[1m]"]);
        assert_eq!(s.options(&f, Field::Effort), ["max", "low"]);
        let gone = Favorite {
            program: "Renamed".into(),
            ..f.clone()
        };
        assert!(s.options(&gone, Field::Model).is_empty());
        assert!(s.options(&gone, Field::Effort).is_empty());
        assert_eq!(s.change(&f, Field::Place, "Studio").place, "Studio");
        assert_eq!(s.change(&f, Field::Effort, "max").effort, "max");
        let plain = s.change(&f, Field::Program, "Plain");
        assert_eq!(
            (
                plain.program.as_str(),
                plain.model.as_str(),
                plain.effort.as_str()
            ),
            ("Plain", "", "")
        );
        assert_eq!(s.change(&f, Field::Program, "Unknown"), f);
        for field in Field::ALL {
            assert_eq!(Field::from_key(field.key()), Some(field));
            assert_eq!(s.change(&f, field, f.get(field)), f);
        }
        assert_eq!(Field::from_key("folder"), None);
    }

    /// Every word quoted after filling in, in one pass; the task one line, behind `--`; no task, no `--`.
    #[test]
    fn the_script_quotes_every_word() {
        let s = settings();
        let f = Favorite {
            place: "Acme".into(),
            program: "Tool".into(),
            model: "k3[1m]".into(),
            effort: "max".into(),
        };
        let (place, program) = s.resolve(&f).unwrap();
        let script = start_script(place, program, &f, "Fix it's\n\"bug\" $(rm)  now");
        assert_eq!(
            script,
            "rm -f -- \"$0\"\ncd '/home/test/acme' || exit 1\nexec 'launch' '--model' 'k3[1m]' '-c' 'effort=max' \
             'acme' 'tool' '--' 'Fix it'\\''s \"bug\" $(rm) now'\n"
        );
        let empty = start_script(place, program, &f, "  \n ");
        assert!(empty.ends_with("'acme' 'tool'\n"), "{empty}");
        let mut odd = place.clone();
        odd.values.insert("context".into(), "{model}'".into());
        let again = start_script(&odd, program, &f, "x");
        assert!(
            again.contains(" '{model}'\\''' "),
            "values are not filled in twice: {again}"
        );
    }

    /// The script through a real `/bin/sh`: whatever the task holds arrives as one argument, exactly, and nothing in it
    /// runs (plan review 2026-11-10: `$(…)`, backticks, quotes, a backslash before a quote, a trailing backslash, a
    /// leading dash). The script is gone afterwards.
    #[test]
    fn the_task_survives_a_real_shell() {
        let dir = std::env::temp_dir().join(format!("lookout-launch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let place = Place {
            name: "Here".into(),
            folder: dir.to_string_lossy().into_owned(),
            values: BTreeMap::new(),
        };
        let program = Program {
            name: "Echo".into(),
            models: vec![],
            efforts: vec![],
            command: words(&["/bin/sh", "-c", ECHO, "", "--", TASK]),
            models_command: vec![],
            model_labels: BTreeMap::new(),
        };
        let f = Favorite {
            place: "Here".into(),
            program: "Echo".into(),
            ..Favorite::default()
        };
        for task in [
            "$(printf INJECTED)",
            "`printf INJECTED`",
            "it's \"quoted\"",
            "\\'; printf INJECTED; #",
            "ends with \\",
            "--version",
            "ä · 😀 {model} {task}",
        ] {
            let path = dir.join("start.sh");
            std::fs::write(&path, start_script(&place, &program, &f, task)).unwrap();
            let out = std::process::Command::new("/bin/sh")
                .arg(&path)
                .output()
                .unwrap();
            assert_eq!(
                String::from_utf8_lossy(&out.stdout),
                format!("--|{task}|"),
                "{task}"
            );
            assert!(!path.exists(), "the script removes itself");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_a_plain_path_reaches_the_shell() {
        assert_eq!(
            launch_exec("/var/folders/x2/T/lookout-new-1-2.sh").as_deref(),
            Some("/bin/sh /var/folders/x2/T/lookout-new-1-2.sh")
        );
        assert_eq!(launch_exec("/tmp/a b.sh"), None);
        assert_eq!(launch_exec("/tmp/a';x.sh"), None);
    }
}

//! A new team from the inbox's + (Kevin 2026-11-10): its lead and, if wanted, its key account start in windows of
//! their own, named "<team> · Team lead" and "<team> · Key account" (`team::member_name`), each with its role text
//! before the task. The lead starts the workers itself. Only programs that can name their session (`{name}`) take a
//! role, Lookout knows the team by the names.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::i18n::tr;
use crate::launch::{
    Favorite, Field, NewSession, favorite_from_key, favorite_key, fill, start_script_named,
};
use crate::team::{Role, member_name};

/// The role texts (`newSession.team`: `lead`, `leadAlone`, `keyAccount`). `{team}`, `{lead}` and `{keyAccount}` stand
/// for the team's name and the names of its two windows, `{worker}` (`{workerProgram}`, `{workerModel}`,
/// `{workerEffort}`) for what the lead starts its workers with. `lead_alone` is the lead's text in a team without a key
/// account. ponytail: three fixed texts; per-team texts when somebody needs them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamTexts {
    pub lead: String,
    pub lead_alone: String,
    pub key_account: String,
}

impl TeamTexts {
    pub fn defaults() -> TeamTexts {
        TeamTexts {
            lead: "You lead the team {team}. Plan the work in steps and give each step to a worker in its own window \
                   named \"{team} · <step>\", started with {worker}. Do not build anything yourself. The workers \
                   report to you. Keep a log of decisions in the user's words. Never ask the user yourself: send every \
                   question to the window \"{keyAccount}\". At 80 % context hand over to a successor named \"{lead}\" \
                   again, with a section \"open threads\", then close this window. The task:"
                .into(),
            lead_alone: "You lead the team {team}. Plan the work in steps and give each step to a worker in its own \
                         window named \"{team} · <step>\", started with {worker}. Do not build anything yourself. The \
                         workers report to you and send their questions to you, never to the user. Ask the user all \
                         open questions together in one dialog. Keep a log of decisions in the user's words. At 80 % \
                         context hand over to a successor named \"{lead}\" again, with a section \"open threads\", \
                         then close this window. The task:"
                .into(),
            key_account: "You are the key account of the team {team}, the only window that asks the user. The lead \
                          \"{lead}\" and the workers send you their questions. Gather them, drop repeats, write them \
                          in plain words with a recommendation and ask them together in one dialog. Pass each answer \
                          back word for word to the window that asked, and tell the lead. The team's task:"
                .into(),
        }
    }

    /// `newSession.team`; a text missing or empty keeps its default.
    pub fn parse(v: Option<&Value>) -> TeamTexts {
        let mut t = TeamTexts::defaults();
        let Some(o) = v.and_then(Value::as_object) else {
            return t;
        };
        let get = |key: &str| {
            o.get(key)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
        };
        if let Some(s) = get("lead") {
            t.lead = s;
        }
        if let Some(s) = get("leadAlone") {
            t.lead_alone = s;
        }
        if let Some(s) = get("keyAccount") {
            t.key_account = s;
        }
        t
    }
}

/// How the + starts a team: the lead's and the key account's place, program, model and effort (both in the lead's
/// place), and whether there is a key account (Kevin 2026-11-10, "1a": a tick in the dialog).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamSetup {
    pub lead: Favorite,
    pub key_account: Favorite,
    /// What the lead starts its workers with (Kevin 2026-11-11: "welche Subscription da genutzt wird"); any program, the
    /// lead reads it from its text (`{worker}`, `{workerProgram}`, `{workerModel}`, `{workerEffort}`).
    pub worker: Favorite,
    pub with_key_account: bool,
}

impl TeamSetup {
    pub fn get(&self, role: Role) -> &Favorite {
        match role {
            Role::KeyAccount => &self.key_account,
            Role::Worker => &self.worker,
            Role::Lead => &self.lead,
        }
    }
}

/// One window of a new team: what the log and the menu call it, its folder and its start script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TeamStart {
    pub label: String,
    pub folder: String,
    pub script: String,
}

impl NewSession {
    /// The programs that can take a team role.
    fn team_programs(&self) -> impl Iterator<Item = &crate::launch::Program> {
        self.programs.iter().filter(|p| p.takes_name())
    }

    fn first_team_favorite(&self) -> Favorite {
        let Some(p) = self.team_programs().next() else {
            return Favorite::default();
        };
        Favorite {
            place: self.places[0].name.clone(),
            ..Favorite::default()
        }
        .with_program(p)
    }

    /// The setup the app stored (`team_setup_to_json`), the first place and naming program for what is missing; with a
    /// key account unless stored otherwise.
    pub fn team_setup(&self, stored: Option<&str>) -> TeamSetup {
        let parsed = stored.and_then(|s| serde_json::from_str::<Value>(s).ok());
        let o = parsed.as_ref().and_then(Value::as_object);
        let role = |key: &str| {
            o.and_then(|o| o.get(key))
                .and_then(|v| favorite_from_key(&v.to_string()))
                .unwrap_or_else(|| self.first_team_favorite())
        };
        TeamSetup {
            lead: role("lead"),
            key_account: role("keyAccount"),
            worker: o
                .and_then(|o| o.get("worker"))
                .and_then(|v| favorite_from_key(&v.to_string()))
                .unwrap_or_else(|| Favorite::default().with_program(&self.programs[0])),
            with_key_account: o
                .and_then(|o| o.get("withKeyAccount"))
                .and_then(Value::as_bool)
                .unwrap_or(true),
        }
    }

    /// What `field` of a role can be: the places, the programs (lead and key account: only naming ones), or the models
    /// or efforts of its program.
    pub fn team_options(&self, role: Role, f: &Favorite, field: Field) -> Vec<&str> {
        match field {
            Field::Program if role != Role::Worker => {
                self.team_programs().map(|p| p.name.as_str()).collect()
            }
            _ => self.options(f, field),
        }
    }

    /// `setup` with `field` of `role` set to `value`; the place is the team's, both roles move. A program that cannot
    /// name its session changes nothing.
    pub fn change_team(
        &self,
        setup: &TeamSetup,
        role: Role,
        field: Field,
        value: &str,
    ) -> TeamSetup {
        let mut out = setup.clone();
        if field == Field::Place {
            out.lead.place = value.to_owned();
            out.key_account.place = value.to_owned();
            return out;
        }
        if field == Field::Program
            && role != Role::Worker
            && !self.program(value).is_some_and(|p| p.takes_name())
        {
            return out;
        }
        let changed = self.change(setup.get(role), field, value);
        match role {
            Role::KeyAccount => out.key_account = changed,
            Role::Worker => out.worker = changed,
            Role::Lead => out.lead = changed,
        }
        out
    }

    /// "Program · model · effort" of a role, the model's name from the daily list.
    pub fn role_label(&self, f: &Favorite) -> String {
        self.label(&Favorite {
            place: String::new(),
            ..f.clone()
        })
    }

    /// The windows to start for team `team` with `task`: the lead, then the key account if wanted. `Err` (the text the
    /// app logs and shows) when a role's choice does not fit the settings any more or its program cannot name the
    /// session.
    pub fn team_starts(
        &self,
        setup: &TeamSetup,
        team: &str,
        task: &str,
    ) -> Result<Vec<TeamStart>, String> {
        let lead_name = member_name(team, Role::Lead);
        let key_name = member_name(team, Role::KeyAccount);
        let worker = self.role_label(&setup.worker);
        let w = &setup.worker;
        let vars: BTreeMap<&str, &str> = [
            ("team", team),
            ("lead", lead_name.as_str()),
            ("keyAccount", key_name.as_str()),
            ("worker", worker.as_str()),
            ("workerProgram", w.program.as_str()),
            ("workerModel", w.model.as_str()),
            ("workerEffort", w.effort.as_str()),
        ]
        .into();
        let mut roles = vec![(
            Role::Lead,
            &lead_name,
            if setup.with_key_account {
                &self.team.lead
            } else {
                &self.team.lead_alone
            },
        )];
        if setup.with_key_account {
            roles.push((Role::KeyAccount, &key_name, &self.team.key_account));
        }
        roles
            .into_iter()
            .map(|(role, name, text)| {
                let f = Favorite {
                    place: setup.lead.place.clone(),
                    ..setup.get(role).clone()
                };
                let (place, program) = self
                    .resolve(&f)
                    .filter(|(_, p)| p.takes_name())
                    .ok_or_else(|| format!("{name}: {}", tr("nicht verfügbar", "not available")))?;
                let text = fill(text, &vars);
                let task = [text.as_str(), task].join(" ");
                Ok(TeamStart {
                    label: format!("{name} ({})", self.label(&f)),
                    folder: place.folder.clone(),
                    script: start_script_named(place, program, &f, &task, name),
                })
            })
            .collect()
    }
}

/// The setup as the app stores it.
pub fn team_setup_to_json(setup: &TeamSetup) -> String {
    let fav = |f: &Favorite| serde_json::from_str::<Value>(&favorite_key(f)).unwrap_or(Value::Null);
    let mut o = Map::new();
    o.insert("lead".into(), fav(&setup.lead));
    o.insert("keyAccount".into(), fav(&setup.key_account));
    o.insert("worker".into(), fav(&setup.worker));
    o.insert("withKeyAccount".into(), json!(setup.with_key_account));
    Value::Object(o).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::launch::parse;

    const HOME: &str = "/home/test";

    fn settings() -> NewSession {
        parse(
            Some(&json!({
                "places": [{"name": "Acme", "folder": "/work/acme"}, {"name": "Studio", "folder": "/work/studio"}],
                "programs": [
                    {"name": "Plain", "command": ["plain", "--", "{task}"]},
                    {"name": "Named", "models": ["big", "small"], "efforts": ["max", "low"],
                     "command": ["tool", "--name", "{name}", "--model", "{model}", "-e", "{effort}", "--", "{task}"]}
                ],
                "team": {"leadAlone": "Alone in {team}, you are {lead}.", "keyAccount": " "}
            })),
            HOME,
        )
    }

    /// Without settings Claude Code names its session for a team, and a plain start leaves the name and its option out.
    #[test]
    fn the_default_program_names_only_team_windows() {
        let d = NewSession::defaults(HOME);
        let setup = d.team_setup(None);
        assert_eq!(setup.lead.program, "Claude Code");
        let (place, program) = d.resolve(&setup.lead).unwrap();
        let plain = crate::launch::start_script(place, program, &setup.lead, "x");
        assert!(
            !plain.contains("--name") && !plain.contains("CLAUDE_SESSION_NAME"),
            "{plain}"
        );
        let starts = d.team_starts(&setup, "Shop", "x").unwrap();
        assert!(
            starts[0].script.contains("'claude' '--name' '"),
            "{}",
            starts[0].script
        );
    }

    #[test]
    fn the_setup_starts_with_a_naming_program_and_comes_back_from_storage() {
        let s = settings();
        let setup = s.team_setup(None);
        assert_eq!(setup.lead.program, "Named", "Plain cannot name its session");
        assert_eq!(
            (setup.lead.model.as_str(), setup.with_key_account),
            ("big", true)
        );
        let changed = s.change_team(&setup, Role::KeyAccount, Field::Model, "small");
        let changed = s.change_team(&changed, Role::Lead, Field::Place, "Studio");
        assert_eq!(
            s.change_team(&changed, Role::Lead, Field::Program, "Plain"),
            changed
        );
        let changed = TeamSetup {
            with_key_account: false,
            ..changed
        };
        assert_eq!(s.team_setup(Some(&team_setup_to_json(&changed))), changed);
        assert_eq!(
            changed.key_account.place, "Studio",
            "the place is the team's"
        );
        assert_eq!(
            s.team_options(Role::Lead, &changed.lead, Field::Program),
            ["Named"]
        );
        assert_eq!(
            s.team_options(Role::Worker, &changed.worker, Field::Program),
            ["Plain", "Named"],
            "workers may use any program"
        );
        let workers = TeamSetup {
            with_key_account: true, // the default lead text names the workers' program
            ..s.change_team(&changed, Role::Worker, Field::Program, "Plain")
        };
        assert_eq!(workers.worker.program, "Plain");
        let starts = s.team_starts(&workers, "Shop", "x").unwrap();
        assert!(
            starts[0].script.contains("started with Plain."),
            "the lead learns what to start workers with: {}",
            starts[0].script
        );
        assert_eq!(s.role_label(&changed.key_account), "Named · small · max");
    }

    #[test]
    fn a_team_starts_its_lead_and_key_account_with_their_names_and_texts() {
        let s = settings();
        let setup = s.team_setup(None);
        let starts = s.team_starts(&setup, "Shop", "Build it's shop").unwrap();
        assert_eq!(starts.len(), 2);
        let lead = member_name("Shop", Role::Lead);
        assert!(
            starts[0]
                .script
                .contains(&format!("export CLAUDE_SESSION_NAME='{lead}'\nexec ")),
            "a naming hook keeps the name: {}",
            starts[0].script
        );
        assert!(
            starts[0].script.contains(&format!("'--name' '{lead}'")),
            "{}",
            starts[0].script
        );
        assert!(
            starts[0].script.contains("\"Shop · Key account\""),
            "the default lead text names the key account"
        );
        assert!(
            starts[0].script.ends_with("Build it'\\''s shop'\n"),
            "the task after the text"
        );
        assert!(starts[1].script.contains("'--name' 'Shop · Key account'"));
        assert!(
            starts[1]
                .script
                .contains("You are the key account of the team Shop"),
            "an empty text keeps the default"
        );
        assert_eq!(starts[0].folder, "/work/acme");

        let alone = TeamSetup {
            with_key_account: false,
            ..setup.clone()
        };
        let starts = s.team_starts(&alone, "Shop", "").unwrap();
        assert_eq!(starts.len(), 1);
        assert!(
            starts[0]
                .script
                .contains(&format!("'Alone in Shop, you are {lead}.'"))
        );

        let gone = TeamSetup {
            lead: Favorite {
                model: "gone".into(),
                ..setup.lead.clone()
            },
            ..setup
        };
        assert!(s.team_starts(&gone, "Shop", "x").is_err());
    }
}

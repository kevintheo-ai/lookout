//! Teams (Kevin 2026-11-10: "jedes team ... eine eigene karte ... wie ein eigener arbeitsbereich"): a lead, a key
//! account if wanted, and workers, each a window named "<team> · <role or task>". A team exists once a window is named
//! "<team> · Team lead" (also "Team-Lead", or a `leadNames` entry) or "<team> · Key account", and stays while any
//! window with its name in front is open, so workers keep their team after the lead closed.

use std::collections::{BTreeMap, BTreeSet};

use crate::i18n::tr;
use crate::model::{TermWindow, Time, fold_case};

/// Between the team and the rest of a window's name.
pub const SEP: &str = " · ";

/// The longest team name the + accepts.
pub const NAME_MAX: usize = 40;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Lead,
    KeyAccount,
    Worker,
}

/// The name a window of `role` gets when the + starts a team.
pub fn member_name(team: &str, role: Role) -> String {
    let word = match role {
        Role::Lead => tr("Team-Lead", "Team lead"),
        Role::KeyAccount => "Key account",
        Role::Worker => "",
    };
    format!("{team}{SEP}{word}")
}

/// The own names of a window's visible session: Kevin's title in Lookout (`Session::title`), the transcript's own name,
/// then the status file's.
fn names(w: &TermWindow) -> Vec<&str> {
    let Some(s) = w.visible_session() else {
        return vec![];
    };
    if let Some(t) = s.title.as_deref() {
        // Kevin's title decides alone (Kevin 2026-11-11): without " · " the window stands on its own.
        return vec![t];
    }
    [
        s.digest.as_ref().and_then(|d| d.name.as_deref()),
        Some(s.name.as_str()),
    ]
    .into_iter()
    .flatten()
    .filter(|n| !n.trim().is_empty())
    .collect()
}

/// The window's name that decides its team: the first own name with a separator. Only this one counts everywhere, so a
/// status file that still holds an old name founds no second team (review quick 2026-11-10).
fn team_name(w: &TermWindow) -> Option<&str> {
    names(w).into_iter().find(|n| split(n).is_some())
}

/// The part of the window's team name in front of " · ": its project, a card can be made for it.
pub fn project(w: &TermWindow) -> Option<&str> {
    split(team_name(w)?).map(|(p, _)| p)
}

/// Is `name` one of `list`, any case and a hyphen as a space (`key`)?
pub fn listed(list: &[String], name: &str) -> bool {
    list.iter().any(|t| key(t) == key(name))
}

/// The name a window gets when the + starts it on card `card` (Kevin 2026-11-11, "3a"): the card, then the task's first
/// words, at most 40 characters of them, a longer first word cut; "new" without a task.
pub fn card_member_name(card: &str, task: &str) -> String {
    let mut words = String::new();
    for w in task.split_whitespace() {
        let space = usize::from(!words.is_empty());
        if words.chars().count() + space + w.chars().count() > NAME_MAX {
            if words.is_empty() {
                words = w.chars().take(NAME_MAX).collect();
            }
            break;
        }
        if !words.is_empty() {
            words.push(' ');
        }
        words.push_str(w);
    }
    if words.is_empty() {
        words = tr("neu", "new").to_owned();
    }
    format!("{card}{SEP}{words}")
}

/// "Team · rest" split at the first separator; both parts trimmed and not empty.
fn split(name: &str) -> Option<(&str, &str)> {
    let (team, rest) = name.split_once(SEP)?;
    let (team, rest) = (team.trim(), rest.trim());
    (!team.is_empty() && !rest.is_empty()).then_some((team, rest))
}

/// A name as names compare: any case, a hyphen as a space, runs of spaces as one ("Acme · Team-Lead" is "acme · team
/// lead", like "ACME · Team lead").
fn key(name: &str) -> String {
    fold_case(name)
        .replace('-', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The role of a window named `name` (`rest` after the team): a `leadNames` entry or "Team lead" leads, "Key account"
/// talks to the user, anything else works.
fn role(name: &str, rest: &str, lead_names: &[String]) -> Role {
    if lead_names.iter().any(|l| l == name) {
        return Role::Lead;
    }
    match key(rest).as_str() {
        "team lead" => Role::Lead,
        "key account" => Role::KeyAccount,
        _ => Role::Worker,
    }
}

/// Keeps `known` up to date: a team stays while a window with its name in front is open, and a lead or key account
/// founds one. Teams compare by `key`; the first spelling seen stays.
pub fn update_known(known: &mut BTreeSet<String>, windows: &[TermWindow], lead_names: &[String]) {
    let mut open = BTreeSet::new();
    let mut founders = vec![];
    for n in windows.iter().filter_map(team_name) {
        let (team, rest) = split(n).expect("team_name splits");
        open.insert(key(team));
        if role(n, rest, lead_names) != Role::Worker {
            founders.push(team);
        }
    }
    known.retain(|t| open.contains(&key(t)));
    for team in founders {
        if !known.iter().any(|t| key(t) == key(team)) {
            known.insert(team.to_owned());
        }
    }
}

/// The team and role of a window, if its team name's part in front is a known team.
pub fn team_of<'a>(
    w: &TermWindow,
    known: &'a BTreeSet<String>,
    lead_names: &[String],
) -> Option<(&'a str, Role)> {
    member(w, known, lead_names).map(|(team, role, _)| (team, role))
}

/// `team_of` with the window's team name.
fn member<'a, 'w>(
    w: &'w TermWindow,
    known: &'a BTreeSet<String>,
    lead_names: &[String],
) -> Option<(&'a str, Role, &'w str)> {
    let n = team_name(w)?;
    let (team, rest) = split(n)?;
    let team = known.iter().find(|t| key(t) == key(team))?;
    Some((team.as_str(), role(n, rest, lead_names), n))
}

/// A team with the windows (indices into `windows`) of each role, every group in the order given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Team {
    pub name: String,
    pub lead: Vec<usize>,
    pub key_account: Vec<usize>,
    pub workers: Vec<usize>,
    /// Older windows under the same name as a newer one, a lead that handed over for example: to be closed.
    pub stale: Vec<usize>,
    /// A project's own card (Kevin 2026-11-11, "3a"), no team: its area groups by state, not by role.
    pub card: bool,
}

impl Team {
    /// All its windows.
    pub fn members(&self) -> impl Iterator<Item = usize> + '_ {
        self.lead
            .iter()
            .chain(&self.key_account)
            .chain(&self.workers)
            .chain(&self.stale)
            .copied()
    }
}

/// The teams and project cards among `indices`, sorted by name. Of windows with the same name only the newest keeps its
/// role, the others are stale (a lead that handed over keeps its name, the successor takes it); a card has no
/// successors, all its windows stay. A card whose project is a team is that team.
pub fn teams(
    windows: &[TermWindow],
    indices: &[usize],
    known: &BTreeSet<String>,
    cards: &[String],
    lead_names: &[String],
) -> Vec<Team> {
    let mut all = known.clone();
    for c in cards {
        if !all.iter().any(|t| key(t) == key(c)) {
            all.insert(c.clone());
        }
    }
    let known_list: Vec<String> = known.iter().cloned().collect();
    let mut out = groups(windows, indices, &all, lead_names);
    for t in &mut out {
        t.card = !listed(&known_list, &t.name);
        if t.card {
            // Two starts without a task are both "<card> · new" (review quick 2026-11-11).
            t.workers.append(&mut t.stale);
        }
    }
    out
}

fn groups(
    windows: &[TermWindow],
    indices: &[usize],
    known: &BTreeSet<String>,
    lead_names: &[String],
) -> Vec<Team> {
    let found: Vec<(usize, &str, Role, &str)> = indices
        .iter()
        .filter_map(|&i| {
            let (team, role, name) = member(&windows[i], known, lead_names)?;
            Some((i, team, role, name))
        })
        .collect();
    // The newest window of each name keeps its role.
    let mut newest: BTreeMap<String, (Time, usize)> = BTreeMap::new();
    for &(i, _, _, name) in &found {
        let at = started(&windows[i]);
        let e = newest.entry(key(name)).or_insert((at, i));
        if at > e.0 {
            *e = (at, i);
        }
    }
    let mut by_team: BTreeMap<&str, Team> = BTreeMap::new();
    for (i, team, role, name) in found {
        let t = by_team.entry(team).or_insert_with(|| Team {
            name: team.to_owned(),
            ..Team::default()
        });
        let group = if newest[&key(name)].1 != i {
            &mut t.stale
        } else {
            match role {
                Role::Lead => &mut t.lead,
                Role::KeyAccount => &mut t.key_account,
                Role::Worker => &mut t.workers,
            }
        };
        group.push(i);
    }
    by_team.into_values().collect()
}

/// When a window's session started; the window's first sighting without one.
fn started(w: &TermWindow) -> Time {
    w.visible_session()
        .and_then(|s| s.started_at)
        .unwrap_or(w.first_seen)
}

/// A team name the + may use: trimmed, not empty, without "·", at most `NAME_MAX` characters, and no open team of that
/// name (any case). `Err` is the text the dialog shows.
pub fn check_name(name: &str, known: &BTreeSet<String>) -> Result<String, &'static str> {
    let name = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if name.is_empty() {
        return Err(tr(
            "Bitte einen Teamnamen eingeben.",
            "Please enter a team name.",
        ));
    }
    if name.contains('·') {
        return Err(tr(
            "Der Teamname darf kein „·“ enthalten.",
            "The team name must not contain \"·\".",
        ));
    }
    if name.chars().count() > NAME_MAX {
        return Err(tr(
            "Der Teamname ist zu lang (höchstens 40 Zeichen).",
            "The team name is too long (40 characters at most).",
        ));
    }
    if known.iter().any(|t| key(t) == key(&name)) {
        return Err(tr(
            "Ein Team mit diesem Namen läuft schon.",
            "A team with this name is already running.",
        ));
    }
    Ok(name)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Digest;
    use crate::testutil::fake_window;

    /// A window whose session calls itself `own`; its status-file name ("x") has no separator.
    fn named(_id: u64, own: &str, started: Time) -> TermWindow {
        let mut w = fake_window(false, started);
        let s = &mut w.sessions[0];
        s.started_at = Some(started);
        s.digest = Some(Digest {
            name: Some(own.into()),
            ..Digest::default()
        });
        w
    }

    fn leads() -> Vec<String> {
        vec!["Beta · Boss".into()]
    }

    /// Two leads of different teams whose own names say nothing (the status file names them): neither is the other's
    /// predecessor, the name that made each a member counts (review quick 2026-11-10).
    #[test]
    fn only_the_same_member_name_makes_a_predecessor() {
        let mut w = vec![named(1, "notes", 1.0), named(2, "notes", 2.0)];
        w[0].sessions[0].name = "Acme · Team lead".into();
        w[1].sessions[0].name = "Beta · Team lead".into();
        let mut known = BTreeSet::new();
        update_known(&mut known, &w, &[]);
        let t = teams(&w, &[0, 1], &known, &[], &[]);
        assert_eq!(
            t.iter()
                .map(|t| (t.lead.clone(), t.stale.len()))
                .collect::<Vec<_>>(),
            [(vec![0], 0), (vec![1], 0)]
        );
    }

    /// A project's card has no successors: two windows of the same name both stay on it, two starts without a task are
    /// both "<card> · new" (review quick 2026-11-11).
    #[test]
    fn a_card_keeps_windows_of_the_same_name() {
        let w = vec![named(1, "Harbor · neu", 1.0), named(2, "Harbor · neu", 2.0)];
        let t = teams(&w, &[0, 1], &BTreeSet::new(), &["Harbor".into()], &[]);
        assert_eq!(
            (t[0].card, t[0].stale.len(), t[0].members().count()),
            (true, 0, 2)
        );
    }

    /// Kevin's title decides alone (Kevin 2026-11-11): "Acme · notes" joins team Acme, a title without " · " leaves it.
    #[test]
    fn kevins_title_decides_the_team() {
        let mut w = vec![
            named(1, "Acme · Team lead", 1.0),
            named(2, "Studio · notes", 2.0),
            named(3, "Acme · #3 build", 3.0),
        ];
        w[1].sessions[0].title = Some("Acme · notes".into());
        w[2].sessions[0].title = Some("My build".into());
        let mut known = BTreeSet::new();
        update_known(&mut known, &w, &[]);
        let t = teams(&w, &[0, 1, 2], &known, &[], &[]);
        assert_eq!((t.len(), &t[0].workers[..]), (1, &[1][..]));
    }

    /// Any case and a hyphen do not split a team: a worker named in small letters belongs, a successor named
    /// "Team-Lead" replaces a "Team lead", and an old name in the status file founds no second team (review quick
    /// 2026-11-10).
    #[test]
    fn spellings_do_not_split_a_team() {
        let mut w = vec![
            named(1, "Acme · Team lead", 1.0),
            named(2, "acme · build", 2.0),
            named(3, "ACME · Team-Lead", 3.0),
            named(4, "Beta · Team lead", 4.0),
        ];
        w[3].sessions[0].name = "Gamma · Team lead".into();
        let mut known = BTreeSet::new();
        update_known(&mut known, &w, &[]);
        assert_eq!(
            known.iter().map(String::as_str).collect::<Vec<_>>(),
            ["Acme", "Beta"]
        );
        let t = teams(&w, &[0, 1, 2, 3], &known, &[], &[]);
        assert_eq!(
            (&t[0].lead[..], &t[0].workers[..], &t[0].stale[..]),
            (&[2][..], &[1][..], &[0][..])
        );
        assert!(check_name("acme", &known).is_err());
    }

    #[test]
    fn a_lead_or_key_account_founds_a_team_and_workers_keep_it() {
        let mut known = BTreeSet::new();
        let w = vec![
            named(1, "Acme · Team-Lead", 1.0),
            named(2, "Acme · #3 build", 2.0),
            named(3, "Studio · notes", 3.0),
            named(4, "Shop · key ACCOUNT", 4.0),
            named(5, "Beta · Boss", 5.0),
        ];
        update_known(&mut known, &w, &leads());
        assert_eq!(
            known.iter().map(String::as_str).collect::<Vec<_>>(),
            ["Acme", "Beta", "Shop"],
            "a name with a separator alone is no team"
        );
        update_known(&mut known, &w[1..2], &leads());
        assert_eq!(known.len(), 1, "the worker keeps Acme, the others are gone");
        update_known(&mut known, &w[2..3], &leads());
        assert!(known.is_empty(), "no window of Acme left");
    }

    #[test]
    fn roles_groups_and_a_handed_over_lead() {
        let w = vec![
            named(1, "Acme · Team lead", 10.0),
            named(2, "Acme · Key account", 11.0),
            named(3, "Acme · #3 build", 12.0),
            named(4, "Acme · Team lead", 20.0),
            named(5, "Studio · notes", 13.0),
            named(6, "Beta · Boss", 14.0),
        ];
        let mut known = BTreeSet::new();
        update_known(&mut known, &w, &leads());
        let t = teams(&w, &[0, 1, 2, 3, 4, 5], &known, &[], &leads());
        assert_eq!(t.len(), 2);
        assert_eq!(
            (
                &t[0].name[..],
                &t[0].lead[..],
                &t[0].key_account[..],
                &t[0].workers[..],
                &t[0].stale[..]
            ),
            ("Acme", &[3][..], &[1][..], &[2][..], &[0][..]),
            "the newer lead leads, the older one is to be closed"
        );
        assert_eq!(
            (&t[1].name[..], &t[1].lead[..]),
            ("Beta", &[5][..]),
            "leadNames"
        );
        assert_eq!(t[0].members().count(), 4);
    }

    #[test]
    fn the_name_the_plus_accepts() {
        let known: BTreeSet<String> = ["Acme".to_owned()].into();
        assert_eq!(check_name("  Big   shop ", &known), Ok("Big shop".into()));
        assert!(check_name(" ", &known).is_err());
        assert!(check_name("acme", &known).is_err(), "any case");
        assert!(check_name("A · B", &known).is_err());
        assert!(check_name(&"x".repeat(41), &known).is_err());
        assert_eq!(member_name("Shop", Role::KeyAccount), "Shop · Key account");
        assert_eq!(
            card_member_name("Harbor", "  Fix  the   ring view now "),
            "Harbor · Fix the ring view now"
        );
        assert_eq!(card_member_name("Harbor", ""), "Harbor · neu");
        assert_eq!(
            card_member_name("R", &"word ".repeat(30)).chars().count(),
            "R · ".chars().count() + 39,
            "at most 40 characters of the task"
        );
        // A first word of 40 characters or a long link is the name, cut to 40, not "new" (review quick 2026-11-11).
        let long = "x".repeat(40);
        assert_eq!(card_member_name("R", &long), format!("R · {long}"));
        assert_eq!(
            card_member_name("R", &format!("{long}yz more")),
            format!("R · {long}")
        );
    }
}

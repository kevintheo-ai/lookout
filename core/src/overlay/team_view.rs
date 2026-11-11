//! A team in the inbox (Kevin 2026-11-10): its tile on the overview and its own area with lead, key account, workers
//! and the windows to close.

use super::{
    InboxInput, InboxTab, InboxTile, ListLine, count_word, inbox_card, menu_row, window_tone,
};
use crate::i18n::tr;
use crate::model::CardButton;
use crate::model::LabelTone;
use crate::team::Team;

/// The team's tile: how many windows; who asks, else how lead and key account are; how many workers run. Amber while
/// one of them asks.
pub(super) fn tile(o: &InboxInput, t: &Team) -> InboxTile {
    let tone = |i: usize| window_tone(&o.windows[i]);
    let asker = t.members().find(|&i| tone(i) == LabelTone::Waiting);
    let count = |of: LabelTone| {
        t.members()
            .filter(|i| !t.stale.contains(i))
            .filter(|&i| tone(i) == of)
            .count()
    };
    let first = match asker {
        Some(i) => format!(
            "? {} {}",
            menu_row(&o.windows[i], o.since, o.lookout, o.now).name,
            tr("fragt dich", "asks you")
        ),
        // A project's card has no roles: how many are done, and stopped ones apart (review quick 2026-11-11).
        None if t.card => {
            let done = tr("fertig", "done");
            let mut parts = vec![count_word(count(LabelTone::Ready), done, done)];
            let stopped = count(LabelTone::Stopped);
            if stopped > 0 {
                let word = tr("angehalten", "stopped");
                parts.push(count_word(stopped, word, word));
            }
            parts.join(" · ")
        }
        None => {
            let mut parts = vec![match t.lead.first() {
                Some(&i) => format!("{} Lead", tone(i).symbol()),
                None => tr("kein Lead", "no lead").to_owned(),
            }];
            if let Some(&i) = t.key_account.first() {
                parts.push(format!("{} Key account", tone(i).symbol()));
            }
            parts.join(" · ")
        }
    };
    // A team counts its workers, a card all its windows but those to close.
    let counted: Vec<usize> = if t.card {
        t.members().filter(|i| !t.stale.contains(i)).collect()
    } else {
        t.workers.clone()
    };
    let running = counted
        .iter()
        .filter(|&&i| matches!(tone(i), LabelTone::Running | LabelTone::Watching))
        .count();
    let second = if t.card {
        count_word(
            running,
            tr("arbeitet", "working"),
            tr("arbeiten", "working"),
        )
    } else {
        count_word(
            running,
            tr("Arbeiter läuft", "worker running"),
            tr("Arbeiter laufen", "workers running"),
        )
    };
    InboxTile {
        tab: InboxTab::Team,
        team: Some(t.name.clone()),
        title: t.name.clone(),
        count: t.members().count(),
        lines: vec![first, second],
        hot: asker.is_some(),
    }
}

/// The team's area: a group per role and the windows to close, each card with its model under the name instead of the
/// team (the head names it).
pub(super) fn area(o: &InboxInput, t: &Team, width: f64) -> Vec<ListLine> {
    let tone = |i: &usize| window_tone(&o.windows[*i]);
    let by_tone = |tones: &[LabelTone]| -> Vec<usize> {
        t.members()
            .filter(|i| !t.stale.contains(i) && tones.contains(&tone(i)))
            .collect()
    };
    // A project's card groups by state like Waiting and Running (Kevin 2026-11-11, "3a"); a team by role.
    let groups: Vec<(&str, Vec<usize>)> = if t.card {
        vec![
            (
                tr("Fragen an dich", "Questions for you"),
                by_tone(&[LabelTone::Waiting]),
            ),
            (tr("Angehalten", "Stopped"), by_tone(&[LabelTone::Stopped])),
            (
                tr("Arbeitet", "Working"),
                by_tone(&[LabelTone::Running, LabelTone::Watching]),
            ),
            (tr("Fertig", "Done"), by_tone(&[LabelTone::Ready])),
            (tr("Zum Schliessen", "To close"), t.stale.clone()),
        ]
    } else {
        vec![
            (tr("Team-Lead", "Team lead"), t.lead.clone()),
            ("Key account", t.key_account.clone()),
            (tr("Arbeiter", "Workers"), t.workers.clone()),
            (tr("Zum Schliessen", "To close"), t.stale.clone()),
        ]
    };
    let mut lines = vec![];
    for (label, group) in groups.into_iter().filter(|(_, g)| !g.is_empty()) {
        lines.push(ListLine::Group(label.to_owned()));
        for i in group {
            let mut card = inbox_card(i, width, o);
            if let Some(ListLine::Card(c)) = card.first_mut() {
                c.project = o.windows[i]
                    .visible_session()
                    .and_then(|s| s.digest.as_ref())
                    .and_then(|d| d.model.as_deref())
                    .map(model_name);
                if t.card {
                    c.menu.push(CardButton::DissolveCard);
                }
            }
            lines.extend(card);
        }
    }
    lines
}

/// "claude-opus-5-5" as "Opus 5.5": the family with its version, a date after it dropped. Any other name stays.
pub(super) fn model_name(model: &str) -> String {
    let short = model.strip_prefix("claude-").unwrap_or(model);
    let parts: Vec<&str> = short
        .split('-')
        .filter(|p| !(p.len() == 8 && p.bytes().all(|b| b.is_ascii_digit())))
        .collect();
    match parts.split_first() {
        Some((family, numbers))
            if !family.is_empty()
                && !numbers.is_empty()
                && numbers
                    .iter()
                    .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())) =>
        {
            let mut chars = family.chars();
            let first: String = chars
                .next()
                .into_iter()
                .flat_map(char::to_uppercase)
                .collect();
            format!("{first}{} {}", chars.as_str(), numbers.join("."))
        }
        _ => model.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use super::model_name;
    use crate::geom::Rect;
    use crate::model::{Digest, ListEntry, SideList, TermWindow};
    use crate::overlay::{
        INBOX_TILE_X, InboxNav, InboxTab, InboxTile, ListLine, inbox_target, nav_at,
    };
    use crate::planned::NaiveDate;
    use crate::team;
    use crate::testutil::fake_window_with;

    /// A window named `name`, started at `started`; asks when `asks`, else works.
    fn win(id: u64, name: &str, asks: bool, started: f64) -> TermWindow {
        let mut w = fake_window_with(asks, 1000.0 + id as f64, None, asks, true);
        w.id = id;
        w.sessions[0].started_at = Some(started);
        w.sessions[0].digest = Some(Digest {
            name: Some(name.into()),
            model: Some("claude-opus-5-5".into()),
            question: asks.then(|| "Ja oder nein?".into()),
            answer_at: Some(500.0),
            ..Digest::default()
        });
        w
    }

    fn windows() -> Vec<TermWindow> {
        vec![
            win(0, "Acme · Team lead", false, 20.0),
            win(1, "Acme · Key account", true, 11.0),
            win(2, "Acme · #3 build", false, 12.0),
            win(3, "Acme · Team lead", false, 10.0),
            win(4, "Studio · notes", false, 13.0),
            win(5, "Beta · Team-Lead", false, 14.0),
        ]
    }

    fn inbox(w: &[TermWindow], tab: InboxTab, open: Option<&str>) -> Vec<ListLine> {
        inbox_with(w, tab, open, &[])
    }

    fn inbox_with(
        w: &[TermWindow],
        tab: InboxTab,
        open: Option<&str>,
        cards: &[String],
    ) -> Vec<ListLine> {
        let mut known = BTreeSet::new();
        team::update_known(&mut known, w, &[]);
        let (since, lookout, read, none) = (
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeMap::new(),
            BTreeSet::new(),
        );
        let planned: Vec<u64> = w.iter().map(|x| x.id).collect();
        let list = SideList {
            rect: Rect::new(0.0, 0.0, 430.0, 900.0),
            entries: (0..w.len()).map(ListEntry::Row).collect(),
            inbox: true,
        };
        let day = NaiveDate::from_ymd_opt(2026, 11, 10).unwrap();
        let o = crate::overlay::InboxInput {
            planned: &planned,
            windows: w,
            since: &since,
            lookout: &lookout,
            now: 1000.0,
            read: &read,
            main: None,
            tab,
            snoozed: &none,
            pinned: &[],
            plan: None,
            plan_day: day,
            today: day,
            teams: &known,
            cards,
            lead_names: &[],
            team: open,
        };
        inbox_target(&list, &o, 7).lines
    }

    fn tiles(lines: &[ListLine]) -> Vec<Vec<&InboxTile>> {
        lines
            .iter()
            .filter_map(|l| match l {
                ListLine::Tiles(t) => Some(t.iter().collect()),
                _ => None,
            })
            .collect()
    }

    /// Below Waiting a tile per team, amber while one of its windows asks; team windows are not counted under Running.
    #[test]
    fn the_overview_has_a_tile_per_team() {
        let w = windows();
        let lines = inbox(&w, InboxTab::Overview, None);
        let rows = tiles(&lines);
        assert_eq!(rows.iter().map(Vec::len).collect::<Vec<_>>(), [1, 2, 2]);
        let (acme, beta) = (rows[1][0], rows[1][1]);
        assert_eq!(
            (
                acme.title.as_str(),
                acme.count,
                acme.hot,
                acme.team.as_deref()
            ),
            ("Acme", 4, true, Some("Acme"))
        );
        assert_eq!(acme.lines, ["? Key account fragt dich", "1 Arbeiter läuft"]);
        assert_eq!(beta.lines, ["● Lead", "0 Arbeiter laufen"]);
        assert!(!beta.hot);
        let run = rows[2][0];
        assert_eq!(
            (run.tab, run.count),
            (InboxTab::Working, 1),
            "only Studio runs outside a team"
        );
        assert_eq!(
            rows[0][0].count, 0,
            "the asking key account stays in its team (Kevin 2026-11-11), its tile is amber"
        );
        let x = INBOX_TILE_X + 1.0;
        let y = 44.0 + 86.0 + 8.0 + 10.0;
        assert_eq!(
            nav_at(&lines, 430.0, x, y),
            Some(InboxNav::Team("Acme".into()))
        );
    }

    /// The team's area: a group per role with the model under the name, the lead that handed over to close.
    #[test]
    fn the_team_area_groups_by_role() {
        let w = windows();
        let lines = inbox(&w, InboxTab::Team, Some("Acme"));
        assert_eq!(
            lines[0],
            ListLine::InboxHead {
                tab: InboxTab::Team,
                counts: crate::overlay::InboxCounts {
                    waiting: 0,
                    working: 1,
                    parked: 0,
                    planned: None
                },
                team: Some(("Acme".into(), 4)),
            }
        );
        let shown: Vec<String> = lines
            .iter()
            .filter_map(|l| match l {
                ListLine::Group(g) => Some(format!("[{g}]")),
                ListLine::Card(c) => Some(format!(
                    "{} {}",
                    c.name,
                    c.project.clone().unwrap_or_default()
                )),
                _ => None,
            })
            .collect();
        assert_eq!(
            shown,
            [
                "[Team-Lead]",
                "Team lead Opus 5.5",
                "[Key account]",
                "Key account Opus 5.5",
                "[Arbeiter]",
                "#3 build Opus 5.5",
                "[Zum Schliessen]",
                "Team lead Opus 5.5",
            ]
        );
        let gone = inbox(&w, InboxTab::Team, Some("Gone"));
        assert!(matches!(
            gone[0],
            ListLine::InboxHead {
                tab: InboxTab::Overview,
                team: None,
                ..
            }
        ));
    }

    /// A card's tile tells done from stopped (review quick 2026-11-11): a turn broken off with Esc is not done.
    #[test]
    fn a_card_tile_tells_done_from_stopped() {
        let mut w = vec![
            win(0, "Ringtop · a", false, 1.0),
            win(1, "Ringtop · b", false, 2.0),
        ];
        w[0].sessions[0].ready = true;
        w[1].sessions[0].ready = true;
        w[1].sessions[0].stopped = true;
        let lines = inbox_with(&w, InboxTab::Overview, None, &["Ringtop".to_owned()]);
        let card = tiles(&lines)
            .into_iter()
            .flatten()
            .find(|t| t.title == "Ringtop")
            .expect("the card's tile");
        assert_eq!(card.lines[0], "1 fertig · 1 angehalten");
    }

    /// A project's own card (Kevin 2026-11-11, "3a"): its windows leave Waiting and Running, its tile counts done and
    /// working, its area groups by state, its windows can dissolve it; a project without a card offers one.
    #[test]
    fn a_project_card_is_its_own_area() {
        use crate::model::CardButton;
        let mut w = vec![
            win(0, "Ringtop · build", false, 1.0),
            win(1, "Ringtop · asks", true, 2.0),
            win(2, "Studio · notes", false, 3.0),
        ];
        w[0].sessions[0].ready = false;
        let cards = ["ringtop".to_owned()];
        let lines = inbox_with(&w, InboxTab::Overview, None, &cards);
        let rows = tiles(&lines);
        assert_eq!(rows[0][0].count, 0, "the card's question stays on its card");
        let card = rows[1][0];
        assert_eq!(
            (
                card.title.as_str(),
                card.count,
                card.hot,
                card.team.as_deref()
            ),
            ("ringtop", 2, true, Some("ringtop"))
        );
        assert_eq!(card.lines[1], "1 arbeitet");
        assert_eq!(rows[2][0].count, 1, "only Studio runs outside");

        let area = inbox_with(&w, InboxTab::Team, Some("ringtop"), &cards);
        let shown: Vec<String> = area
            .iter()
            .filter_map(|l| match l {
                ListLine::Group(g) => Some(format!("[{g}]")),
                ListLine::Card(c) => Some(c.name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(shown, ["[Fragen an dich]", "asks", "[Arbeitet]", "build"]);
        let menus: Vec<bool> = area
            .iter()
            .filter_map(|l| match l {
                ListLine::Card(c) => Some(c.menu.contains(&CardButton::DissolveCard)),
                _ => None,
            })
            .collect();
        assert_eq!(menus, [true, true]);

        let running = inbox_with(&w, InboxTab::Working, None, &[]);
        let studio = running
            .iter()
            .find_map(|l| match l {
                ListLine::Card(c) if c.name == "notes" => Some(c),
                _ => None,
            })
            .expect("Studio under Running");
        assert!(studio.menu.contains(&CardButton::MakeCard));
    }

    #[test]
    fn model_names() {
        assert_eq!(model_name("claude-opus-5-5"), "Opus 5.5");
        assert_eq!(model_name("claude-fable-5"), "Fable 5");
        assert_eq!(model_name("claude-opus-4-20250514"), "Opus 4");
        assert_eq!(model_name("glm-5.3"), "glm-5.3");
        assert_eq!(model_name("deepseek-v4.1-flash"), "deepseek-v4.1-flash");
        assert_eq!(model_name("-5"), "-5");
    }
}

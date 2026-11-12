//! A team in the inbox (Kevin 2026-11-10): its tile on the overview and its own area with lead, key account, workers
//! and the windows to close.

use super::{
    InboxInput, InboxTab, InboxTile, ListLine, area_read, count_word, inbox_card, is_parked,
    menu_row, pin_rank, window_tone,
};
use crate::i18n::tr;
use crate::model::CardButton;
use crate::model::LabelTone;
use crate::team::Team;

/// What a project card or a team offers on its tile and its cards (Kevin 2026-11-12: "ganze projekte parken", "teams
/// sollten auch geparkt werden können"): to park its windows while one could still be parked (a pinned one stays pinned,
/// one to close stays where it is), to bring them back while one is parked.
fn whole_menu(o: &InboxInput, t: &Team) -> Vec<CardButton> {
    let parked = |i: usize| is_parked(&o.windows[i], o);
    let pinned = |i: usize| pin_rank(&o.windows[i], o.pinned).is_some();
    let (park, unpark) = if t.card {
        (CardButton::ParkProject, CardButton::UnparkProject)
    } else {
        (CardButton::ParkTeam, CardButton::UnparkTeam)
    };
    let mut menu = vec![];
    if t.members()
        .any(|i| !t.stale.contains(&i) && !parked(i) && !pinned(i))
    {
        menu.push(park);
    }
    if t.members().any(parked) {
        menu.push(unpark);
    }
    menu
}

/// The team's tile: how many windows; who asks, else how lead and key account are; how many workers run. Amber while
/// one of them asks that Kevin has not read (Kevin 2026-11-12: a read card "keine Reaktion mehr ... farblich"). What
/// Kevin parked is quiet (Kevin 2026-11-12): it neither asks nor counts here, and a project or team whose every window he
/// parked rests, grey with "pausiert", at the foot of the overview (`overview`).
pub(super) fn tile(o: &InboxInput, t: &Team) -> InboxTile {
    let tone = |i: usize| window_tone(&o.windows[i]);
    let base = InboxTile {
        tab: InboxTab::Team,
        team: Some(t.name.clone()),
        title: t.name.clone(),
        menu: whole_menu(o, t),
        ids: t
            .members()
            .flat_map(|i| o.windows[i].sessions.iter().map(|s| s.id.clone()))
            .collect(),
        ..InboxTile::default()
    };
    let active: Vec<usize> = t
        .members()
        .filter(|&i| !is_parked(&o.windows[i], o))
        .collect();
    let asker = active
        .iter()
        .copied()
        .find(|&i| tone(i) == LabelTone::Waiting && !area_read(&o.windows[i], o));
    // Rests: nothing left to park, something parked (every window pinned is no park), and none of the others asks
    // (review quick 2026-11-12, GLM: a pinned window that asks keeps the tile amber).
    if asker.is_none()
        && matches!(
            base.menu[..],
            [CardButton::UnparkProject | CardButton::UnparkTeam]
        )
    {
        let n = t.members().filter(|&i| is_parked(&o.windows[i], o)).count();
        let parked = tr("geparkt", "parked");
        return InboxTile {
            count: n,
            lines: vec![
                tr("⏸ pausiert", "⏸ paused").to_owned(),
                count_word(n, parked, parked),
            ],
            paused: true,
            ..base
        };
    }
    let count = |of: LabelTone| {
        active
            .iter()
            .filter(|i| !t.stale.contains(i))
            .filter(|&&i| tone(i) == of)
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
    let counted: Vec<usize> = active
        .iter()
        .copied()
        .filter(|i| {
            if t.card {
                !t.stale.contains(i)
            } else {
                t.workers.contains(i)
            }
        })
        .collect();
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
        count: active.len(),
        lines: vec![first, second],
        hot: asker.is_some(),
        ..base
    }
}

/// Room above "Geparkt" at the least (Kevin 2026-11-12: "einen Abstand zu den laufenden sessions").
const PARKED_GAP_MIN: f64 = 40.0;

/// The team's area: a group per role and the windows to close, each card with its model under the name instead of the
/// team (the head names it). What Kevin parked leaves its group for "Geparkt" (Kevin 2026-11-12: "so dass sie wirklich
/// geparkt ist"), which keeps its distance and starts in the lower third of the `shown` points below the head ("im
/// unteren Viertel oder Drittel"); a window to close stays where it is. What Kevin read goes under "Gelesen" right below
/// the others (Kevin 2026-11-12: "einfach unter allen anderen Karten"), with its state.
pub(super) fn area(o: &InboxInput, t: &Team, width: f64, shown: f64) -> Vec<ListLine> {
    let tone = |i: &usize| window_tone(&o.windows[*i]);
    let parked = |i: &usize| !t.stale.contains(i) && is_parked(&o.windows[*i], o);
    let read = |i: &usize| !t.stale.contains(i) && area_read(&o.windows[*i], o);
    let by_tone = |tones: &[LabelTone]| -> Vec<usize> {
        t.members()
            .filter(|i| !t.stale.contains(i) && !parked(i) && !read(i) && tones.contains(&tone(i)))
            .collect()
    };
    let unparked = |g: &[usize]| -> Vec<usize> {
        g.iter()
            .copied()
            .filter(|i| !parked(i) && !read(i))
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
            (tr("Team-Lead", "Team lead"), unparked(&t.lead)),
            ("Key account", unparked(&t.key_account)),
            (tr("Arbeiter", "Workers"), unparked(&t.workers)),
            (tr("Zum Schliessen", "To close"), t.stale.clone()),
        ]
    };
    let mut lines = vec![];
    for (label, group) in groups.into_iter().filter(|(_, g)| !g.is_empty()) {
        lines.extend(group_of(o, t, width, label, &group));
    }
    let seen: Vec<usize> = t.members().filter(|i| read(i)).collect();
    if !seen.is_empty() {
        lines.extend(group_of(o, t, width, tr("Gelesen", "Read"), &seen));
    }
    let resting: Vec<usize> = t.members().filter(|i| parked(i)).collect();
    if !resting.is_empty() {
        let used: f64 = lines.iter().map(ListLine::height).sum();
        lines.push(ListLine::Gap(parked_gap(used, shown)));
        lines.extend(group_of(o, t, width, tr("Geparkt", "Parked"), &resting));
    }
    lines
}

/// One group of an area: its title, then its cards with the model under the name; a card's windows can dissolve it, and
/// every window can park or bring back the whole project or team.
fn group_of(o: &InboxInput, t: &Team, width: f64, label: &str, group: &[usize]) -> Vec<ListLine> {
    let mut lines = vec![ListLine::Group(label.to_owned())];
    let whole = whole_menu(o, t);
    for &i in group {
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
            c.menu.extend(&whole);
        }
        lines.extend(card);
    }
    lines
}

/// The room above "Geparkt", in whole points: down to the lower third of the `shown` height, never less than
/// `PARKED_GAP_MIN` below what stands above (`used`).
fn parked_gap(used: f64, shown: f64) -> u32 {
    (shown * 2.0 / 3.0 - used).max(PARKED_GAP_MIN).round() as u32
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
        inbox_parked(w, tab, open, cards, &BTreeSet::new())
    }

    /// As `inbox_with`, with the sessions Kevin parked.
    fn inbox_parked(
        w: &[TermWindow],
        tab: InboxTab,
        open: Option<&str>,
        cards: &[String],
        parked: &BTreeSet<String>,
    ) -> Vec<ListLine> {
        inbox_full(w, tab, open, cards, parked, &[])
    }

    /// As `inbox_parked`, with the sessions Kevin pinned.
    fn inbox_full(
        w: &[TermWindow],
        tab: InboxTab,
        open: Option<&str>,
        cards: &[String],
        parked: &BTreeSet<String>,
        pinned: &[String],
    ) -> Vec<ListLine> {
        inbox_all(w, tab, open, cards, parked, pinned, &BTreeMap::new())
    }

    /// As `inbox_full`, with what Kevin marked read.
    fn inbox_all(
        w: &[TermWindow],
        tab: InboxTab,
        open: Option<&str>,
        cards: &[String],
        parked: &BTreeSet<String>,
        pinned: &[String],
        read: &crate::layout::ReadAnswers,
    ) -> Vec<ListLine> {
        let mut known = BTreeSet::new();
        team::update_known(&mut known, w, &[]);
        let (since, lookout) = (BTreeMap::new(), BTreeMap::new());
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
            read,
            main: None,
            tab,
            snoozed: parked,
            pinned,
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
            win(0, "Harbor · a", false, 1.0),
            win(1, "Harbor · b", false, 2.0),
        ];
        w[0].sessions[0].ready = true;
        w[1].sessions[0].ready = true;
        w[1].sessions[0].stopped = true;
        let lines = inbox_with(&w, InboxTab::Overview, None, &["Harbor".to_owned()]);
        let card = tiles(&lines)
            .into_iter()
            .flatten()
            .find(|t| t.title == "Harbor")
            .expect("the card's tile");
        assert_eq!(card.lines[0], "1 fertig · 1 angehalten");
    }

    /// A project's own card (Kevin 2026-11-11, "3a"): its windows leave Waiting and Running, its tile counts done and
    /// working, its area groups by state, its windows can dissolve it; a project without a card offers one.
    #[test]
    fn a_project_card_is_its_own_area() {
        use crate::model::CardButton;
        let mut w = vec![
            win(0, "Harbor · build", false, 1.0),
            win(1, "Harbor · asks", true, 2.0),
            win(2, "Studio · notes", false, 3.0),
        ];
        w[0].sessions[0].ready = false;
        let cards = ["harbor".to_owned()];
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
            ("harbor", 2, true, Some("harbor"))
        );
        assert_eq!(card.lines[1], "1 arbeitet");
        assert_eq!(rows[2][0].count, 1, "only Studio runs outside");

        let area = inbox_with(&w, InboxTab::Team, Some("harbor"), &cards);
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

    /// Groups as "[name]", project headings as "<name>", cards by name.
    fn shown(lines: &[ListLine]) -> Vec<String> {
        lines
            .iter()
            .filter_map(|l| match l {
                ListLine::Group(g) => Some(format!("[{g}]")),
                ListLine::Project(p) => Some(format!("<{p}>")),
                ListLine::Card(c) => Some(c.name.clone()),
                _ => None,
            })
            .collect()
    }

    fn card_named<'a>(lines: &'a [ListLine], name: &str) -> &'a crate::overlay::InboxCard {
        lines
            .iter()
            .find_map(|l| match l {
                ListLine::Card(c) if c.name == name => Some(c),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no card {name}"))
    }

    /// Kevin 2026-11-12: a card parked in a project's area stops its colour, says "pausiert" and goes to "Geparkt" at the
    /// foot of the area ("so dass sie wirklich geparkt ist"); it no longer counts on the tile or in the head.
    #[test]
    fn a_parked_card_is_paused_at_the_foot_of_its_area() {
        let mut w = vec![
            win(0, "Harbor · build", false, 1.0),
            win(1, "Harbor · docs", false, 2.0),
            win(2, "Harbor · notes", false, 3.0),
        ];
        w[1].sessions[0].ready = true;
        w[2].sessions[0].ready = true;
        let cards = ["harbor".to_owned()];
        let parked: BTreeSet<String> = [w[2].sessions[0].id.clone()].into();
        let area = inbox_parked(&w, InboxTab::Team, Some("harbor"), &cards, &parked);
        assert_eq!(
            shown(&area),
            [
                "[Arbeitet]",
                "build",
                "[Fertig]",
                "docs",
                "[Geparkt]",
                "notes"
            ]
        );
        let notes = card_named(&area, "notes");
        assert!(notes.paused && notes.since.as_deref() == Some("pausiert"));
        assert!(!card_named(&area, "docs").paused);
        // Kevin 2026-11-12: "einen Abstand zu den laufenden sessions ... im unteren Viertel oder Drittel".
        let k = area
            .iter()
            .position(|l| *l == ListLine::Group("Geparkt".into()))
            .expect("Geparkt");
        assert!(
            matches!(area[k - 1], ListLine::Gap(_)),
            "room above the parked ones"
        );
        let head = if matches!(area[1], ListLine::WaitBar { .. }) {
            2
        } else {
            1
        };
        let shown = 900.0 - area[..head].iter().map(ListLine::height).sum::<f64>();
        let start: f64 = area[head..k].iter().map(ListLine::height).sum();
        assert!(
            (start - shown * 2.0 / 3.0).abs() <= 0.5,
            "in the lower third: {start} of {shown}"
        );
        assert!(
            matches!(
                &area[0],
                ListLine::InboxHead {
                    team: Some((_, 2)),
                    ..
                }
            ),
            "the head counts the two that are not parked"
        );
        let lines = inbox_parked(&w, InboxTab::Overview, None, &cards, &parked);
        let tile = tiles(&lines)
            .into_iter()
            .flatten()
            .find(|t| t.title == "harbor")
            .expect("the card's tile");
        assert_eq!((tile.count, tile.lines[0].as_str()), (2, "1 fertig"));
    }

    /// Kevin 2026-11-12: "ich will auch ganze projekte parken können". Tile and cards offer it; parked, every card rests
    /// under "Geparkt" and the tile, grey with "pausiert", at the foot of the overview, offering the way back. A window
    /// that asks anew comes back, and with it the tile.
    #[test]
    fn a_whole_project_parks_and_comes_back() {
        use crate::model::CardButton;
        let mut w = vec![
            win(0, "Harbor · build", false, 1.0),
            win(1, "Harbor · docs", false, 2.0),
            win(2, "Studio · notes", false, 3.0),
        ];
        w[1].sessions[0].ready = true;
        let cards = ["harbor".to_owned()];
        let harbor = |lines: &[ListLine]| -> InboxTile {
            tiles(lines)
                .into_iter()
                .flatten()
                .find(|t| t.title == "harbor")
                .cloned()
                .expect("the project's tile")
        };
        let mut parked = BTreeSet::new();
        let before = harbor(&inbox_parked(&w, InboxTab::Overview, None, &cards, &parked));
        assert_eq!(before.menu, [CardButton::ParkProject]);
        assert!(!before.paused && before.ids.len() == 2);

        let mut read = crate::layout::ReadAnswers::new();
        crate::layout::park_all(&w, &[0, 1], true, &mut read, &mut parked);
        let overview = inbox_parked(&w, InboxTab::Overview, None, &cards, &parked);
        let after = harbor(&overview);
        assert!(after.paused && !after.hot);
        assert_eq!(after.lines, ["⏸ pausiert", "2 geparkt"]);
        assert_eq!(after.menu, [CardButton::UnparkProject]);
        assert!(
            matches!(overview.last(), Some(ListLine::Tiles(t)) if t[0].title == "harbor"),
            "at the foot of the overview"
        );
        let area = inbox_parked(&w, InboxTab::Team, Some("harbor"), &cards, &parked);
        assert_eq!(shown(&area), ["[Geparkt]", "build", "docs"]);
        assert!(
            card_named(&area, "build")
                .menu
                .contains(&CardButton::UnparkProject)
        );

        w[0].sessions[0].waiting = true;
        let overview = inbox_parked(&w, InboxTab::Overview, None, &cards, &parked);
        assert!(!harbor(&overview).paused, "a new question brings it back");
        crate::layout::park_all(&w, &[0, 1], false, &mut read, &mut parked);
        assert!(parked.is_empty(), "brought back");
    }

    /// Kevin 2026-11-12: "teams sollten auch geparkt werden können". The same as a project, with the team's words.
    #[test]
    fn a_whole_team_parks_too() {
        use crate::model::CardButton;
        let w = vec![
            win(0, "Acme · Team lead", false, 1.0),
            win(1, "Acme · Key account", false, 2.0),
            win(2, "Acme · #3 build", false, 3.0),
        ];
        let tile_of = |parked: &BTreeSet<String>| -> InboxTile {
            tiles(&inbox_parked(&w, InboxTab::Overview, None, &[], parked))
                .into_iter()
                .flatten()
                .find(|t| t.title == "Acme")
                .cloned()
                .expect("the team's tile")
        };
        let mut parked = BTreeSet::new();
        let before = tile_of(&parked);
        assert_eq!(before.menu, [CardButton::ParkTeam]);
        assert_eq!(before.ids.len(), 3);
        let mut read = crate::layout::ReadAnswers::new();
        crate::layout::park_all(&w, &[0, 1, 2], true, &mut read, &mut parked);
        let after = tile_of(&parked);
        assert!(after.paused);
        assert_eq!(after.menu, [CardButton::UnparkTeam]);
        assert_eq!(after.lines, ["⏸ pausiert", "3 geparkt"]);
        let area = inbox_parked(&w, InboxTab::Team, Some("Acme"), &[], &parked);
        assert_eq!(
            shown(&area),
            ["[Geparkt]", "Team lead", "Key account", "#3 build"]
        );
        assert!(
            card_named(&area, "#3 build")
                .menu
                .contains(&CardButton::UnparkTeam)
        );
    }

    /// A pinned window stays pinned when the project parks (review quick 2026-11-12): once the others rest, "park the whole
    /// project" is no longer offered, as it would do nothing, and the tile rests with the number really parked.
    #[test]
    fn a_pinned_window_does_not_keep_a_project_from_resting() {
        use crate::model::CardButton;
        let w = vec![
            win(0, "Harbor · build", false, 1.0),
            win(1, "Harbor · docs", false, 2.0),
        ];
        let cards = ["harbor".to_owned()];
        let parked: BTreeSet<String> = [w[0].sessions[0].id.clone()].into();
        let pinned = [w[1].sessions[0].id.clone()];
        let overview = inbox_full(&w, InboxTab::Overview, None, &cards, &parked, &pinned);
        let tile = tiles(&overview)
            .into_iter()
            .flatten()
            .find(|t| t.title == "harbor")
            .cloned()
            .expect("the project's tile");
        assert_eq!(tile.menu, [CardButton::UnparkProject]);
        assert!(tile.paused);
        assert_eq!(tile.lines[1], "1 geparkt");

        // Every window pinned, none parked: nothing to offer, and the project does not rest.
        let pinned = [w[0].sessions[0].id.clone(), w[1].sessions[0].id.clone()];
        let overview = inbox_full(
            &w,
            InboxTab::Overview,
            None,
            &cards,
            &BTreeSet::new(),
            &pinned,
        );
        let tile = tiles(&overview)
            .into_iter()
            .flatten()
            .find(|t| t.title == "harbor")
            .cloned()
            .expect("the project's tile");
        assert!(tile.menu.is_empty() && !tile.paused);
    }

    /// A pinned window that asks keeps a project or team from resting (review quick 2026-11-12, GLM): everything else
    /// parked, the tile is amber with the question instead of grey, and still offers the way back.
    #[test]
    fn a_pinned_window_that_asks_keeps_the_tile_awake() {
        use crate::model::CardButton;
        let tile_of =
            |w: &[TermWindow], cards: &[String], title: &str, parked: &[usize], asks: usize| {
                let parked: BTreeSet<String> = parked
                    .iter()
                    .map(|&i| w[i].sessions[0].id.clone())
                    .collect();
                let pinned = [w[asks].sessions[0].id.clone()];
                tiles(&inbox_full(
                    w,
                    InboxTab::Overview,
                    None,
                    cards,
                    &parked,
                    &pinned,
                ))
                .into_iter()
                .flatten()
                .find(|t| t.title == title)
                .cloned()
                .unwrap_or_else(|| panic!("no tile {title}"))
            };
        let w = vec![
            win(0, "Harbor · build", false, 1.0),
            win(1, "Harbor · docs", true, 2.0),
        ];
        let tile = tile_of(&w, &["harbor".to_owned()], "harbor", &[0], 1);
        assert!(!tile.paused && tile.hot, "the project's tile is awake");
        assert_eq!(
            (tile.count, tile.lines[0].as_str()),
            (1, "? docs fragt dich")
        );
        assert_eq!(tile.menu, [CardButton::UnparkProject]);

        let w = vec![
            win(0, "Acme · Team lead", false, 1.0),
            win(1, "Acme · Key account", true, 2.0),
            win(2, "Acme · #3 build", false, 3.0),
        ];
        let tile = tile_of(&w, &[], "Acme", &[0, 2], 1);
        assert!(!tile.paused && tile.hot, "the team's tile is awake");
        assert_eq!(
            (tile.count, tile.lines[0].as_str()),
            (1, "? Key account fragt dich")
        );
        assert_eq!(tile.menu, [CardButton::UnparkTeam]);

        // A window to close that asks wakes it too (review quick 2026-11-12, GLM lead): a team's windows never show
        // under Waiting, so resting grey would hide its question.
        let w = vec![
            win(0, "Acme · Team lead", false, 20.0),
            win(1, "Acme · Key account", false, 11.0),
            win(2, "Acme · Team lead", true, 10.0),
        ];
        let parked: BTreeSet<String> =
            [w[0].sessions[0].id.clone(), w[1].sessions[0].id.clone()].into();
        let tile = tiles(&inbox_parked(&w, InboxTab::Overview, None, &[], &parked))
            .into_iter()
            .flatten()
            .find(|t| t.title == "Acme")
            .cloned()
            .expect("the team's tile");
        assert!(!tile.paused && tile.hot, "the old lead asks");
        assert_eq!(tile.lines[0], "? Team lead fragt dich");
        assert_eq!(tile.menu, [CardButton::UnparkTeam]);
    }

    /// The room above "Geparkt": down to the lower third of what the area shows, at least 40 pt in a long area.
    #[test]
    fn parked_cards_keep_their_distance() {
        assert_eq!(super::parked_gap(208.0, 820.0), 339);
        assert_eq!(
            super::parked_gap(500.0, 300.0),
            40,
            "a long area: the least room"
        );
    }

    /// The same in a team's area: a parked worker leaves its role for "Geparkt" and no longer runs on the tile.
    #[test]
    fn a_parked_team_window_leaves_its_role() {
        let w = windows();
        let parked: BTreeSet<String> = [w[2].sessions[0].id.clone()].into();
        let area = inbox_parked(&w, InboxTab::Team, Some("Acme"), &[], &parked);
        assert_eq!(
            shown(&area),
            [
                "[Team-Lead]",
                "Team lead",
                "[Key account]",
                "Key account",
                "[Zum Schliessen]",
                "Team lead",
                "[Geparkt]",
                "#3 build",
            ]
        );
        assert!(card_named(&area, "#3 build").paused);
        let lines = inbox_parked(&w, InboxTab::Overview, None, &[], &parked);
        let acme = tiles(&lines)
            .into_iter()
            .flatten()
            .find(|t| t.title == "Acme")
            .expect("the team's tile");
        assert_eq!(acme.lines[1], "0 Arbeiter laufen");
    }

    /// Kevin 2026-11-12 (a test, kept after he tried it): Waiting's "Fertig" also lists the done cards of each project
    /// card, every project under its own heading below the loose ones, and they count as waiting. Parked ones stay away,
    /// teams keep theirs.
    #[test]
    fn done_cards_of_projects_stand_under_their_name_in_waiting() {
        let mut w = vec![
            win(0, "Mail · check", false, 1.0),
            win(1, "Harbor · docs", false, 2.0),
            win(2, "Harbor · build", false, 3.0),
            win(3, "Studio · notes", false, 4.0),
            win(4, "Harbor · old", false, 5.0),
            win(5, "Acme · Team lead", false, 6.0),
        ];
        for i in [0, 1, 3, 4, 5] {
            w[i].sessions[0].ready = true;
        }
        let cards = ["harbor".to_owned(), "studio".to_owned()];
        let parked: BTreeSet<String> = [w[4].sessions[0].id.clone()].into();
        let lines = inbox_parked(&w, InboxTab::Waiting, None, &cards, &parked);
        assert_eq!(
            shown(&lines),
            ["[Fertig]", "check", "<harbor>", "docs", "<studio>", "notes"]
        );
        assert!(matches!(
            lines[0],
            ListLine::InboxHead {
                counts: crate::overlay::InboxCounts { waiting: 3, .. },
                ..
            }
        ));
        assert_eq!(card_named(&lines, "check").project.as_deref(), Some("Mail"));
        assert_eq!(
            card_named(&lines, "docs").project,
            None,
            "the heading names the project"
        );
        w.remove(0);
        let lines = inbox_parked(&w, InboxTab::Waiting, None, &cards, &parked);
        assert_eq!(
            shown(&lines),
            ["[Fertig]", "<harbor>", "docs", "<studio>", "notes"],
            "without a loose one the group still heads them"
        );
    }

    /// Kevin 2026-11-12: "die Karte ist gelesen. Dann soll sie aber auch dort bleiben bei sich drinnen ... einfach unter
    /// allen anderen Karten ... mit der Überschrift ... gelesen", showing its state as before, and on the overview it no
    /// longer colours the tile. It comes back as soon as it does something new (Kevin: "a", the first way).
    #[test]
    fn a_read_card_stays_in_its_area_under_gelesen() {
        use crate::layout::{ReadAnswers, press_button};
        use crate::model::{CardButton, LabelTone};
        let mut w = vec![
            win(0, "Harbor · build", false, 1.0),
            win(1, "Harbor · asks", true, 2.0),
            win(2, "Harbor · docs", false, 3.0),
            win(3, "Harbor · notes", false, 4.0),
        ];
        w[2].sessions[0].ready = true;
        let cards = ["harbor".to_owned()];
        let mut parked: BTreeSet<String> = [w[3].sessions[0].id.clone()].into();
        let mut read = ReadAnswers::new();
        let area = |w: &[TermWindow], read: &ReadAnswers, parked: &BTreeSet<String>| {
            inbox_all(w, InboxTab::Team, Some("harbor"), &cards, parked, &[], read)
        };
        let tile = |w: &[TermWindow], read: &ReadAnswers, parked: &BTreeSet<String>| {
            tiles(&inbox_all(
                w,
                InboxTab::Overview,
                None,
                &cards,
                parked,
                &[],
                read,
            ))
            .into_iter()
            .flatten()
            .find(|t| t.title == "harbor")
            .cloned()
            .expect("the project's tile")
        };
        let menu = |lines: &[ListLine], name: &str| card_named(lines, name).menu.clone();
        let before = area(&w, &read, &parked);
        assert!(
            menu(&before, "asks").contains(&CardButton::MarkRead),
            "a question can be read"
        );
        assert!(
            menu(&before, "docs").contains(&CardButton::MarkRead)
                && !menu(&before, "docs").contains(&CardButton::ReadNext),
            "a done card too, without walking on"
        );
        assert!(
            !menu(&before, "build").contains(&CardButton::MarkRead),
            "nothing to read while it works"
        );
        assert!(tile(&w, &read, &parked).hot);

        let none = BTreeSet::new();
        for i in [1, 2] {
            let next = press_button(
                CardButton::MarkRead,
                &w,
                &none,
                Some(i),
                i,
                &mut read,
                &mut parked,
            );
            assert_eq!(next, None, "moves nothing");
        }
        let after = area(&w, &read, &parked);
        assert_eq!(
            shown(&after),
            [
                "[Arbeitet]",
                "build",
                "[Gelesen]",
                "asks",
                "docs",
                "[Geparkt]",
                "notes"
            ]
        );
        let k = after
            .iter()
            .position(|l| *l == ListLine::Group("Gelesen".into()))
            .expect("Gelesen");
        assert!(
            matches!(after[k - 1], ListLine::Card(_)),
            "right below the others, no gap"
        );
        let asks = card_named(&after, "asks");
        assert!(
            asks.tone == LabelTone::Waiting && !asks.paused,
            "it still shows its question"
        );
        assert!(
            asks.menu.contains(&CardButton::Unpark) && !asks.menu.contains(&CardButton::MarkRead)
        );
        assert!(
            !tile(&w, &read, &parked).hot,
            "the tile no longer reacts to it"
        );

        // Kevin answers: it works again, no longer done since then (review quick 2026-11-12, Muse lead), and comes back.
        let mut working = w.clone();
        working[1].sessions[0].waiting = false;
        working[1].sessions[0].ready = false;
        working[1].ready_since = None;
        assert_eq!(
            shown(&area(&working, &read, &parked)),
            [
                "[Arbeitet]",
                "build",
                "asks",
                "[Gelesen]",
                "docs",
                "[Geparkt]",
                "notes"
            ]
        );

        w[1].sessions[0].digest.as_mut().unwrap().answer_at = Some(600.0);
        assert_eq!(
            shown(&area(&w, &read, &parked))[..2],
            ["[Fragen an dich]", "asks"],
            "something new brings it back"
        );
        assert!(tile(&w, &read, &parked).hot);
        press_button(
            CardButton::Unpark,
            &w,
            &none,
            None,
            2,
            &mut read,
            &mut parked,
        );
        assert!(
            shown(&area(&w, &read, &parked)).contains(&"[Fertig]".to_owned()),
            "brought back"
        );

        // The same in a team's area, below its windows to close; its tile stays quiet.
        let w = windows();
        let (mut read, mut snoozed) = (ReadAnswers::new(), BTreeSet::new());
        press_button(
            CardButton::MarkRead,
            &w,
            &none,
            None,
            1,
            &mut read,
            &mut snoozed,
        );
        let area = inbox_all(&w, InboxTab::Team, Some("Acme"), &[], &snoozed, &[], &read);
        assert_eq!(
            shown(&area),
            [
                "[Team-Lead]",
                "Team lead",
                "[Arbeiter]",
                "#3 build",
                "[Zum Schliessen]",
                "Team lead",
                "[Gelesen]",
                "Key account",
            ]
        );
        let overview = inbox_all(&w, InboxTab::Overview, None, &[], &snoozed, &[], &read);
        let acme = tiles(&overview)
            .into_iter()
            .flatten()
            .find(|t| t.title == "Acme")
            .cloned()
            .expect("the team's tile");
        assert!(!acme.hot);
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

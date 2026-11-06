# Lookout

I run a lot of AI sessions in the terminal at the same time, and I kept losing track of them. Which one is
done? Which one has been waiting for my answer for a while? So I built Lookout with the help of AI: a small Mac
app for exactly that.

It sits in the menu bar, lines up the terminal windows of your Claude Code, Codex, Grok and Kimi sessions in
Warp or Ghostty, and shows which session is working, which one is done and which one is waiting for you.

![Lookout in action: a finished session gets a mint frame and glides to the left, a session with a question box gets an amber frame and goes first, one click brings it to the middle, and a session that only waits for its CI run turns grey](docs/lookout-demo.gif)

*Animation with demo sessions. It follows Lookout's real rules, just faster.*

## What it does

![Lookout's Focus layout as a still picture: the active session in the middle, a session waiting for an answer and a finished one on the left, two busy sessions and one watching session on the right](docs/focus-layout.png)

- **Frames:** when a session is done, its window gets a soft mint frame. When it can't go on without you,
  because it needs your approval or wants you to pick an answer in a question box, the frame is amber. If the AI
  only asks something at the end of its answer, that counts as done; the label then shows the first sentence of
  the answer, so open the window to see the question.
  The window you're working in never gets a frame; you're looking at it anyway.
- **Labels:** every other AI window gets a small dark label with the status and how long it has been in it, the
  session name and, for Claude Code, the current step while it works, and once a turn is over the question, the
  session's own status line (see below) or the first sentence of the answer. The
  name is the session's own name (`/rename`, `claude --name`, or set by a hook); Claude's automatic topic title
  changes as the session goes on, so Lookout never shows it and shows the folder instead. You see it all without
  clicking into the window. For Codex, Grok and Kimi the label is shorter: status, time and window title.
- **Four states:** `? WAITING FOR YOU`, `✓ DONE`, `● WORKING` and `◌ WATCHING` (Claude Code only, see below).
- **Layouts:** three layouts for all AI windows on one screen, switched with Ctrl+Opt+1, 2 and 3.
  - **Focus:** the window you click goes to the middle. The others line up by who needs you first: waiting, then
    done (first come, first served), then working.
  - **Focus 2:** the middle stays yours, waiting and done windows on the left, working ones on the right. On
    screens narrower than about 2360 points (every MacBook) only one strip fits (on the right by default): it
    holds the waiting and done windows, and the working sessions become rows in a list there.
  - **Grid:** every window gets a fixed place (2 × 2 up to 5 × 2, only the sizes that fit). Nothing moves when a
    status changes, and a click doesn't change the order.
- **Control room:** with two or more screens, your active session and the windows that need you are on one
  screen, the busy ones on the other. There are three variants in the menu. The layout is fixed there, so the
  hotkeys don't apply.
- **Windows that come to you:** in Focus, Focus 2 and the control room, a window glides to the front of the
  line as soon as its session is done or needs you, and back once it works again. In the Grid every window keeps
  its place.
- **Windows stay where Lookout puts them:** if you drag an arranged window somewhere else, Lookout moves it back
  within a few seconds. To keep a window where you put it, take it out under *More > Choose windows*, or use
  *More > Pause (move nothing)*.
- **Screens:** every connected display shows up in the menu under the name macOS gives it ("All on DELL
  U2723QE", "All on the MacBook"; two identical monitors get (1) and (2) from the left). Lookout remembers the
  display you chose. While it's unplugged, the windows go to your biggest remaining screen, and they come back
  when you plug it in again. With three or more screens, *Control room screens* lets you pick which one holds your
  active session and which one shows the busy ones.
- **Lists at the side:** in Focus and Focus 2, when a side is full, the extra sessions show up as rows in a list
  at the bottom of that side. Their windows wait behind the center window; a click on a row brings that window
  to the center. The lists only show while labels are on.
- **Parking lot (optional):** with the setting `workerNames`, the windows of worker sessions wait behind your
  active session while they work or watch, and come back when they are done or ask something. Details below.
- **Menu bar:** the icon's top right tile fills as soon as a session is done or waiting (the window you're working
  in counts too). No numbers, nothing blinking. The menu lists your sessions by status; a click brings that window
  to the front and makes it the active one. Hover over a screen or layout in the menu to see where your windows
  would go before anything moves.
- **The menu:** it is a real macOS menu (keyboard, VoiceOver, light and dark mode as usual). The sessions stand in
  sections with a count: WAITING FOR YOU, DONE, WORKING (working and watching sessions) and, with a parking lot,
  IN THE PARKING LOT. That last section is compact: three rows, then "… and N more", which opens the peek. Each
  session row has a colored dot (amber: waiting for you, mint: done, grey: working, grey ring: watching, paler on
  the parking lot), the session name, below it the project and what the session does or asks, and on the right how
  long it has been in that state. Hover over a session to see where its window is. Screens, layouts, the parking
  lot, *More* and *Quit* have symbols. The switches (labels, strip side, pause) and the window choices (*Choose
  windows*, *Without window*) are under *More*.
- **"Watching":** some Claude Code sessions are done but only wait for their own background job (a CI run,
  workers), for another session, or for a timer they set themselves. They turn grey instead of "done": they don't
  need you, so they don't ask for your attention. Details and limits below.

Lookout only moves windows with a recognized AI session. Normal terminal windows, minimized and full screen
windows, and windows you took out stay where they are.

## How it was built

I built Lookout myself, with the help of AI. I am the architect and the brain: I design it, plan it, set the
rules, decide and accept the result. The AI agents carry out the work: Claude Code writes the code, first in
Swift, now in Rust. Several language models from different vendors review plans and code in rounds: Codex, GLM
and Kimi reviewed the Swift version; Codex, GLM, Muse and Grok reviewed the Rust version. This is not vibe coding.
I used it every day and said what had to change until it felt right. The "watching" rules were tuned on
1,204 real cases from my own sessions. The status rules were carried over unchanged: on 41,293 real answer texts
from my sessions, the Rust version's word rules give exactly the same results as the Swift version's. I work in
German, so the code comments and the log file are in German too.

Version 4.0 is a rewrite of Lookout in Rust. It keeps the behavior of the Swift version and adds the parking lot
and a new look for the menu. The code is a Cargo workspace with two crates:

- `core`: all the logic (settings, layouts, parking lot and peek, status rules, matching windows to sessions, log
  format). It makes no operating system calls; CI also runs its tests on Linux.
- `macos`: the Mac layer through the objc2 crates (Accessibility, window list, processes, the title marker,
  gliding windows, frames, labels, lists, the hover preview, menu and hotkeys).

The Swift version stays in the git history, under the tag `swift-final`.

## Requirements

- macOS 26 or later (I use it on macOS 27)
- [Warp](https://www.warp.dev) (the stable version, not Warp Preview) or [Ghostty](https://ghostty.org). I use
  Warp every day; Ghostty works too, but it got less real use.
- Claude Code (tested with 2.1.295) and/or the Codex, Grok or Kimi command line tools
- Rust 1.88 or newer ([rustup.rs](https://rustup.rs), or `brew install rust`), and the Xcode Command Line Tools
  (`xcode-select --install`) for the macOS SDK and the linker

Lookout reads Claude Code's status from its files. For background agents and as a fallback it also runs
`claude agents`, and it looks for `claude` in `~/.local/bin`, `/opt/homebrew/bin` and `/usr/local/bin`. If yours
lives elsewhere (nvm, for example), add a symlink: `ln -s "$(which claude)" ~/.local/bin/claude`.

## Install

```sh
git clone https://github.com/kevintheo-ai/lookout.git
cd lookout
./build.sh
open ~/Applications/Lookout.app
```

On first start macOS asks for **Accessibility** access (System Settings > Privacy & Security >
Accessibility > Lookout). Lookout needs it to read and move terminal windows; nothing else. Right after that it
lines up your AI windows in Focus on your biggest external screen (or on the Mac's own screen if there is no
other). You can pick another screen or layout in the menu.

To start Lookout at login: System Settings > General > Login Items > add Lookout.

`./build.sh --tests` runs a format check, the linter (clippy) and the tests: 188 right now. If you have Xcode.app
installed and the build complains about the Xcode license, run `sudo xcodebuild -license accept` once.

`./build.sh --app` builds and signs the app into `target/Lookout.app` without installing it, for a test next to
the installed one.

**Keep the Accessibility access across rebuilds (optional):** `build.sh` signs ad hoc if you have no code
signing identity, and macOS then asks for access again after every rebuild. To avoid that, create a
self-signed certificate (Keychain Access > Certificate Assistant > Create a Certificate…, Certificate Type:
Code Signing), check that `security find-identity -v -p codesigning` lists it, and build once with
`LOOKOUT_SIGN_ID="<certificate name>" ./build.sh`. Later builds reuse the identity of the installed app.

`build.sh` also understands `LOOKOUT_APP_DIR` (default `~/Applications`) and `LOOKOUT_BUNDLE_ID`
(default: the installed app's, else `io.github.kevintheo-ai.lookout`). It builds for the CPU of your Mac.

## Update and uninstall

- **Update:** `git pull && ./build.sh`, then quit Lookout from its menu and open it again.
- **Uninstall:** quit Lookout, delete `~/Applications/Lookout.app`, remove it from Accessibility and Login
  Items. Optional leftovers: `~/.config/lookout.json`, `~/Library/Logs/Lookout.log*` and the preferences
  (`defaults delete io.github.kevintheo-ai.lookout`).

## Settings

Everything works without a settings file. To change something, create `~/.config/lookout.json` and restart
Lookout (it reads the file once at start), for example:

```json
{ "language": "en", "claudeConfigDirs": ["~/.claude", "~/.claude-work"], "glowColor": "#5ECFA0" }
```

| Key | Default | Meaning |
|---|---|---|
| `language` | your Mac's language | `"de"` or `"en"` (German or English menus and labels) |
| `claudeConfigDirs` | `["~/.claude"]` | Claude Code configuration folders to watch, one per `CLAUDE_CONFIG_DIR` you use |
| `layout` | `"reading"` | layout until you choose one in the menu: `reading` (Focus), `focus2`, `grid` |
| `stackSide` | `"right"` | side of the window column (the "strip" in the menu) when only one fits next to the center: `right`, `left` |
| `readyThresholdSec` | `90` | seconds of quiet before a background command counts as resting, and the fallback for tools that don't say when their answer ends |
| `pollSec` | `3` | how often windows are checked (seconds) |
| `sessionPollSec` | `6` | how often session status is read (seconds) |
| `hotkeys` | `true` | Ctrl+Opt+1 … 3 for Focus, Focus 2 and the last grid; with `workerNames` also Ctrl+Opt+4 (peek) and Ctrl+Opt+5 (parking lot by hand) |
| `glow` | `true` | colored frames around windows that are done or waiting |
| `glowColor` | `"#5ECFA0"` | frame color for "done" (mint) |
| `glowColorWaiting` | `"#E8B25C"` | frame color for "waiting for you" (amber) |
| `labels` | `true` | whether labels show at first start; after that, your choice in the menu is used |
| `leadNames` | `[]` | own names (`claude --name …` or `/rename`) of sessions that lead other sessions, written exactly the same; an automatic topic title does not count (see "Watching" below); these sessions never park |
| `otherWindowNames` | `[]` | parts of window names where you answer for other sessions (for example `"front desk"`, any case); a `?` status line that only sends you there does not bring this window forward (see "The session's own status line") |
| `workerNames` | `[]` | parts of session names (any case) that mark worker windows for the parking lot, for example `"Worker · "`; without them there is no parking lot |

The menu remembers your choice of screen, layout, grid, control room screens, excluded windows and labels, and
also which windows you sent to the parking lot or took off it by hand.
Pause and the side of the strip (*More > Strip on the right/left*) start fresh after a restart.

## What Lookout does on your Mac

I want you to know exactly what it touches:

- It **moves AI terminal windows** as soon as it has Accessibility access. *More > Pause (move nothing)* stops
  all moving until you turn it off or quit Lookout. Frames and labels keep updating.
- To learn which window belongs to which session, it briefly writes a marker into the session's **tab title**
  and then puts the title back. You may see a title flash. A tab in the background can't be read, so it gets its
  folder name as title instead. For sessions it couldn't match yet, Lookout tries again every 2 minutes.
- It reads the **status files and the end of the transcripts** that Claude Code keeps in its configuration
  folder, and the session logs of Codex (`~/.codex`), Grok (`~/.grok`) and Kimi (`~/.kimi-code`). It runs
  `claude agents` every 5 minutes, and more often if the status files are missing.
- Some sessions **start with a prompt you didn't type** (for example a handover from another session). To tell
  these apart from your own messages, it reads the start arguments of that Claude process.
- It opens **no network connection** itself. It runs local tools only: `nettop` and `lsof` (only while a Claude
  session has finished its answer but still runs background commands), and `claude`. It reads the process list
  directly.
- It writes a log to `~/Library/Logs/Lookout.log` (in German; rotated at 5 MB, one old file kept). The log
  contains session names, status changes and the titles of terminal windows you click, but no answer text.
- The **labels show transcript text** (answers, questions), and so does the open menu. Before you share your
  screen, turn the labels off (*More > Show labels*) and keep the menu closed.

## "Watching" in detail

Lookout is strict here: a grey session that actually needed your answer is worse than one "done" too many. So
a Claude Code session shows grey **WATCHING** instead of **DONE** only when its answer is done and

- Claude Code reports that only background commands are left (status `shell`), a background command of the
  session rests (no real CPU or network use for 90 seconds) and is not a server (a server listens on a port; a
  resting server alone means "done"), or
- it only waits for another running Claude session that talks to it, or for a timer it set itself, and its
  last answer says explicitly that nothing is needed from you,

and the answer contains no question, no request to you, no earlier question still open ("my question above
stands", "still open with you") and no "finished" message, and the turn was not started by your own message
(with the exceptions below).

A few details:

- "Done" about a part ("part 1 is done"), negated ("not finished"), still to come ("when the run is done") or
  as a "Done:" heading next to "Still to do:" is not a "finished" message.
- With a resting background command it is enough that the first sentence says explicitly that nothing is
  needed from you and the answer contains no question; what follows may report on other windows.
- If your own message started the turn, a resting background command still means "watching" when its last
  sentence is a promise to report back ("I'll report back when …", "I'll let you know") and the answer
  contains no question, no request or condition for you ("please", "if you") and no "finished" message.
- With a resting background command, once you have had the session's tab in front after the answer (clicked
  it, or terminal in front with that tab while Lookout checked, screen unlocked), your message no longer
  counts: the answer is judged as if the session had started the turn itself.
- For a session that waits for another session or a timer, the same applies once you have seen the answer,
  but the answer still has to say explicitly that nothing is needed from you; your message does not end its
  conversation with the other session.
- For a session that waits for another session or a timer, "I have nothing to ask you" counts as "nothing
  needed" too (not as the first sentence with a resting background command: no question is not the same as no
  request). If the session also says in a sentence that does not mention you (nor your first name) that it
  waits ("I hold until the other run", "I wait for the lead"), progress words such as "is merged" or "done" are
  not a "finished" message, unless they stand in a sentence that mentions you ("When you're done testing,
  merge PR #12") or a comma, semicolon or dash follows them ("The summary is finished — reply by noon").
  "Close this window" or "handed over" still are.
- If such a waiting session's first sentence says explicitly that nothing is needed from you and it says
  itself that it waits, requests further down no longer count: they are often reports on other windows ("the
  editor waits on your answer"). A "?", an earlier question still open, a "finished" message, "please" or
  "close this window" still turn it "done".
- A prompt the session was started with (`claude "…"`, for example a handover from another session) does not
  count as your message, even if you started it that way yourself.
- A session named in `leadNames` (one that runs other sessions and reports on their windows) with a resting
  background command that is not a server turns "done" only when its answer contains a "?" or your own unseen
  message started the turn.
  Its reports ("two windows wait for you", "the handover is complete") are about other windows, not this one.

When in doubt, Lookout shows "done". Still, on my 1,204 test cases a few sessions that asked for something turned
grey, so it's not perfect.

## The session's own status line

A Claude Code session knows better than any word list whether it needs you. If its answer ends with one of these
lines, Lookout takes that instead of the rules above:

```
◌ Lookout: nothing needed (what it waits for)
? Lookout: request (what you should do)
✓ Lookout: done (the result in a few words)
```

- `?` shows **WAITING FOR YOU**, like an open question, also when only background commands are left. If the
  text in brackets only sends you to a window named in `otherWindowNames` ("approve the plan in the front desk",
  without "here" or "this window"), it counts as `◌`: you answer there, not here. In that window itself it stays `?`.
- `◌` shows **WATCHING** only if something will wake the session on its own (a resting background command,
  another session it talks to, a timer); without that it stays **DONE**, because nothing would continue it.
- `✓` shows **DONE**, even if the answer says "nothing needed".
- Only the last non-empty line counts, and only for the turn it was written in. The sign decides, not the words, so
  any language works. The text in brackets goes on the label.

To have your sessions write the line, give interactive sessions an instruction such as: "End every reply that ends
your turn with exactly one final line for Lookout: `◌ Lookout: nothing needed (…)` only when I have nothing to do
and something else wakes you, `? Lookout: request (…)` when I should do anything, otherwise `✓ Lookout: done (…)`."
I add it with a `SessionStart` and `UserPromptSubmit` hook (`additionalContext`) that skips background runs
(`claude -p`), because scripts read their output. A line in `CLAUDE.md` would reach those runs too.

## The parking lot (optional)

The parking lot is off by default. You turn it on with the setting `workerNames`: parts of session names, any
case, for example `"Worker · "`. A window whose session name contains one of them is a worker.

- While a worker works or watches, its window waits behind your active session: no frame, no label, no place in
  the side columns. When it is done or asks something, it comes back next to your active session like any other
  window; when it works again, it goes back.
- Never parked are sessions named in `leadNames`, the active session, a window with a question in a tab behind
  it, and excluded, minimized and full screen windows.
- It works in Focus, Focus 2 and the control room. The Grid on one screen keeps showing all windows.
- Without `workerNames` nothing changes, and Ctrl+Opt+4 and 5 stay free for other apps.

**Peek:** Ctrl+Opt+4 or *Show parking lot* in the menu shows all parked windows as tiles on your biggest screen,
in front, with labels. The tiles are as large as their number allows and never smaller than Warp's minimum window
size (480 × 192 points). What does not fit stands as rows in a list next to the tiles (a MacBook screen holds 15
tiles, or 10 next to the list). While the peek is open nothing moves, even when workers finish, ask or join. A
click on a tile or a list row brings that worker to the middle at once. Ctrl+Opt+4 again closes the peek and
brings your session back to the front; a click into any other window ends it too. There is no peek during
*More > Pause (move nothing)*, and none with an empty parking lot.

**By hand:** Ctrl+Opt+5 sends the window you are in to the parking lot (it then behaves like a worker) or takes it
back. *To the parking lot* in the menu does the same for any window; a worker can be taken off the parking lot
there too and then stays in the arrangement. Lookout remembers this per session.

## Limits

- Status files and transcript entries are **internal formats of Claude Code**. If the status files change,
  Lookout falls back to `claude agents`, which means one extra query at every status check. If the transcript
  entries change, "watching" no longer appears and labels lose the step and the answer. Older Claude Code versions
  that report background commands as `busy` instead of `shell` show such sessions as working.
- Codex, Grok and Kimi get less: no "watching", no step or answer on the label, and the session name is the
  folder name. A new Codex, Grok or Kimi session shows up after its first turn. Lookout reads their logs only
  in the default folders. I have seen "waiting for an approval" in real use with Grok. For Codex and Kimi I
  haven't seen it yet; Lookout recognizes it by the event name in their logs.
- The word list behind "watching" knows English and German only. In other languages a session with a resting
  background command can turn grey even if its answer asks you for something, as long as there is no "?".
- Only **Warp and Ghostty** windows are arranged. Sessions in other terminals (VS Code, iTerm, Terminal)
  are listed under *More > Without window*. Sessions inside `tmux` are not matched to a window.
- With Warp and Ghostty at the same time, a Ghostty window parked behind a Warp center window can end up in
  front of it (Lookout can only raise windows within one app).
- On narrow screens (a 13-inch MacBook Air, a monitor in portrait mode) the center in Focus gets narrow
  (about 500 points); Focus 2 keeps it wide.
- Identical monitors that report no serial number look the same to macOS; Lookout tells them apart only by their
  order from the left. After you unplug or plug in one of them, your choice can move to its neighbor; one click in
  the menu sets it again.
- A session whose own name contains a `workerNames` entry parks too, even if it is one of yours.
- Parked windows show no sign on screen; the menu and Ctrl+Opt+4 show them.

## Kurz auf Deutsch

Lookout ist eine kleine Menüleisten-App, die ich mit KI-Hilfe für meinen eigenen Schreibtisch gebaut habe: Sie ordnet
die Terminalfenster deiner KI-Sessions (Claude Code, Codex, Grok, Kimi in Warp oder Ghostty) und zeigt ruhig, wer
arbeitet, wer fertig ist und wer auf dich wartet. Fertig ist mintgrün umrahmt, bernsteinfarben heisst: die Session
braucht deine Freigabe oder deine Auswahl in einem Fragefenster. Endet eine Antwort mit einer Lookout-Zeile
(`◌` / `?` / `✓ Lookout: …`, siehe oben), gilt, was die Session selbst sagt. Seit Version 4 ist Lookout in Rust
neu geschrieben. Mit der Einstellung `workerNames` warten die Fenster von Worker-Sessions hinter deiner aktiven
Session (Parkplatz); Ctrl+Opt+4 zeigt sie. Installation wie oben (`git clone`, `./build.sh`,
Bedienungshilfen freigeben). Die Sprache folgt dem Mac; fest auf Deutsch mit `{"language": "de"}` in
`~/.config/lookout.json`. Die Schilder zeigen Text aus deinen Sessions: vor dem Bildschirmteilen unter
*Mehr > Schilder anzeigen* ausschalten.

## License

MIT, see [LICENSE](LICENSE). Lookout is an independent project and not affiliated with Anthropic, OpenAI,
xAI, Moonshot AI, Warp or Ghostty. All product names are trademarks of their respective owners.

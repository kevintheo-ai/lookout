# Lookout

I run a lot of AI sessions in the terminal at the same time, and I kept losing track of them. Which one is
done? Which one has been waiting for my answer for a while? So I built Lookout with the help of AI: a small Mac
app for exactly that.

It sits in the menu bar, lines up the terminal windows of your Claude Code, Codex, Grok and Kimi sessions in
Warp or Ghostty, and shows which session is working, which one is done and which one is waiting for you.

Background story: [Why I run my company with AI agents](https://kevintheo.com/field-report/)

![Lookout in action: a finished session gets a mint frame and glides to the left, a session with a question box gets an amber frame and goes first, one click brings it to the middle, and a session that only waits for its CI run turns grey](docs/lookout-demo.gif)

*Animation with demo sessions. It follows Lookout's real rules, just faster.*

## What it does

![Lookout's Focus layout as a still picture: the active session in the middle, a session waiting for an answer and a finished one on the left, two busy sessions and one watching session on the right](docs/focus-layout.png)

- **Frames:** when a session is done, its window gets a soft mint frame. When it can't go on without you,
  because it needs your approval or wants you to pick an answer in a question box, the frame is amber. If the AI
  only asks something at the end of its answer, that counts as done; the label then shows the first sentence of
  the answer, so open the window to see the question.
  A stopped session, one whose turn you broke off with Esc, gets a calm blue-grey frame.
  The window you're working in never gets a frame; you're looking at it anyway.
- **Labels:** every other AI window gets a small dark label with the status and how long it has been in it, the
  session name and, for Claude Code, the current step while it works, and once a turn is over the question, the
  session's own status line (see below) or the first sentence of the answer. The
  name is the session's own name (`/rename`, `claude --name`, or set by a hook); Claude's automatic topic title
  changes as the session goes on, so Lookout never shows it and shows the folder instead. You see it all without
  clicking into the window. For Codex, Grok and Kimi the label is shorter: status, time and window title.
- **Five states:** `? WAITING FOR YOU`, `⏸ STOPPED`, `✓ DONE`, `● WORKING` and `◌ WATCHING` (Claude Code only, see
  below). `⏸ STOPPED` is a turn that you broke off with Esc. It shows in a calm blue-grey, as its label, as its sign in
  the menu and as its frame, and the label says where it stopped ("stopped at: …", the step) or shows the start of the
  answer. Lookout recognizes it in Claude Code, by the entry "[Request interrupted by user]" in the transcript, and in
  Codex, by the event `turn_aborted` in its log; your next message or a new answer ends it.
- **Layouts:** four layouts for all AI windows on one screen. Focus, Focus 2 and Grid are switched with
  Ctrl+Opt+1, 2 and 3. Inbox is chosen in the menu.
  - **Focus:** the window you click goes to the middle. The others line up by who needs you first: waiting, then
    done (first come, first served), then working.
  - **Focus 2:** the middle stays yours, waiting and done windows on the left, working ones on the right. On
    screens narrower than about 2360 points (every MacBook) only one strip fits (on the right by default): it
    holds the waiting and done windows, and the working sessions become rows in a list there.
  - **Grid:** every window gets a fixed place (2 × 2 up to 5 × 2, only the sizes that fit). Nothing moves when a
    status changes, and a click doesn't change the order.
  - **Inbox:** A window of 430 points holds the Inbox, with its overview and its areas, and the active terminal sits
    right beside it at reading width. The pair is centred on the screen, and the rest of a wide screen stays free. On
    a narrower screen the terminal gets what is left. All other AI windows wait behind the terminal. Inbox has no
    hotkey. You choose it in the menu. It applies to all windows on one screen. In the control room the layout is
    fixed. The whole Inbox window, with its head, tiles, areas and strip, shows only while labels are on. The Inbox
    has an overview with one tile per area. Waiting stands across the top. Below it, Running and Planned stand side by
    side, and Parked stands across below them. The Planned tile comes with the setting `plannedFile`, see the section
    "Planned (optional)". Without that setting, Running and Parked stand side by side. Each tile shows the name of its
    area, its count, and up to two short lines of what is in it. Waiting shows up to two of the sessions that wait,
    with their status sign and name, or "Nobody is waiting for you.". Running shows how many work and how many are in
    the parking lot, or "Nobody is working right now.". Parked shows how many you parked and how many you marked read,
    or "Nothing parked.". A click on a tile opens its area. The areas Waiting, Running and Parked list the same groups
    as the old tabs did. At the top is a head, with the + at its right on the overview and in every area alike. On the
    overview the head shows "Overview". In an area it shows "‹ Overview" at the left and the name of the area with its
    count. A click on "‹ Overview" leads back to the overview. Two fingers swiped sideways on the trackpad, left or
    right, lead back too, like "back" in a browser. The swipe has to go more than twice as far sideways as up or down.
    One swipe leads back once, and the rest of that swipe does not move the overview. The head stays at the top, and
    so does the amber strip while it shows. Everything below them scrolls with the normal macOS scrolling. Scrolling
    up and down works as before, and so does a mouse wheel. Every card is shown. An area you open starts at its top.
    The scroll bar follows the macOS setting, and with "Always" the cards get narrower so nothing is cut off on the
    right. Waiting lists the sessions that want something from you, questions first, then the group "Stopped", then done
    ones. Cards you pinned
    stand above all of them. Within each of the two groups, whoever has waited longest stands on top, and a click on a
    card does not change the order. Running lists the sessions at work, then the parking lot if you use one. Parked
    has two groups, "Parked by you" and "Read". While somebody waits for you, the Waiting tile is amber and its title
    reads "Waiting for you". In every other area, an amber strip stays right under the head. It says how many wait,
    always starting with "?" ("? 1 is waiting for you" or "? 3 are waiting for you"), names the first of them with the
    text of their card (their question, or what a done session answered), and ends with "view ›". A click on the strip
    opens Waiting. Lookout starts on the overview. After that the Inbox shows what you had open last, the overview or
    an area, also when you choose the Inbox again in the menu or turn the labels off and on. The card of the terminal
    in front is open and shows the whole question, up to 6 lines. A click on a card brings that session into the
    terminal. You answer in the terminal as usual. A session counts as seen only once you move on to another card, not
    when you click it. So a card does not leave the list under your mouse. A right-click on a card offers Pin, next to
    Park. On a done card it also offers "Read: hide until the next answer". On a card in the Parked area it offers
    "Bring back", not Pin. On a pinned card it offers only Unpin. The open card in the Parked area also has a "Bring
    back" button. There are no other buttons on the cards. Parking is for a session you do not want to follow, for
    example one that watches a mailbox all day. A parked session stays in the Parked area whatever it does. If it then
    asks something new, it comes to Waiting, and it goes back to Parked once answered. You can park a card with an
    open question too. It leaves Waiting until the session asks something else. For sessions with no answer time in
    their transcript (Codex, Grok) the question stays under Waiting, because a later question could not be told apart.
    A pinned card stands in its own group, Pinned, at the top of the Waiting area, with a pin before its name. Pinned
    cards stand in the order you pinned them. The first one is on top, a new one goes below. A pinned card stays there
    whatever its session does, whether it asks, is done or works, until you choose Unpin. Below the Pinned group,
    Waiting stays as before: questions first, then "Stopped", then done ones, whoever has waited longest on top. A
    pinned session that
    is working is also listed under Running. The count of Waiting, on its tile and in its head, counts a pinned
    session only while it waits for you. Pinning or unpinning a card ends an earlier park or read mark of that card.
    Pinning moves no window. There is no reordering by dragging. Parking and pins stay through a restart of Lookout.
    An open question you parked shows under Waiting again after such a restart. If you park the card in front or mark
    it read, the next waiting session comes to the terminal. Pinned cards are skipped there. Doing the same on another
    card moves nothing. What you marked read is kept in memory only: after quitting Lookout those cards are back. A +
    at the right of the head, on the overview and in every area alike, opens a menu with the three favorites, "Choose
    freely", "New team …", "Empty window" and "Change favorites". "Empty window" does what the + did before: it opens a
    new empty
    window of the terminal in front, Warp in your home folder or Ghostty, and Warp if no terminal is in front, and
    Ghostty needs version 1.3 or later for this. The favorites and "Choose freely" start a new AI session, see "New
    sessions from the +", while "Change favorites" only changes and saves the favorites.
    Teams and project cards get a tile of their own right below Waiting, two tiles side by side. Their windows show only
    in their own area (pinned ones stay in Pinned as well). A right-click on a card also offers "Rename …" and, for a
    window named "Project · …", "Own card for …". More in the section "Teams, project cards and renaming".
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
  sections with a count: WAITING FOR YOU, DONE and, with a parking lot, IN THE PARKING LOT, while working and
  watching sessions sit behind one row "Running" with an arrow and a count, and that submenu lists all of them.
  That last section is compact: three rows, then "… and N more", which opens the peek. Each session row has a
  colored dot (amber: waiting for you, blue-grey pause sign: stopped and listed under WAITING FOR YOU, mint: done, grey:
  working, grey ring: watching, paler on the parking lot),
  the session name, below it the project and what the session does or asks, and on the right how long it has been
  in that state. Hover over a session to see where its window is. Screens, layouts, the parking lot, *More* and
  *Quit* have symbols. The switches (labels, strip side, pause) and the window choices (*Choose windows*, *Without
  window*) are under *More*. *New session* holds the same menu as the + in the Inbox, so a first session can start
  from there too.
- **"Watching":** some Claude Code sessions are done but only wait for their own background job (a CI run,
  workers), for another session, or for a timer they set themselves. They turn grey instead of "done": they don't
  need you, so they don't ask for your attention. Details and limits below.
- **Restore sessions:** after a crash, a shutdown, or when Warp was quit, the menu shows "Restore sessions" with
  a count. You can restore all of them or single ones. Each one opens in its own Warp window and runs the real
  `claude --resume`, 3 seconds apart. The conversation comes back. A command that was running at that moment, and
  background jobs, do not continue by themselves. That is how `claude --resume` works. A session you close
  yourself while Warp keeps running is not offered. "Clear the list" removes all offers. Only Claude Code sessions
  in Warp are offered. Ghostty sessions are not. A session whose transcript no longer exists cannot be restored.
  The setting `resumeCommands` changes the command per Claude configuration folder.

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
| `layout` | `"reading"` | layout until you choose one in the menu: `reading` (Focus), `focus2`, `grid`, `inbox` |
| `stackSide` | `"right"` | side of the window column (the "strip" in the menu) when only one fits next to the center: `right`, `left` |
| `readyThresholdSec` | `90` | seconds of quiet before a background command counts as resting, and the fallback for tools that don't say when their answer ends |
| `pollSec` | `3` | how often windows are checked (seconds) |
| `sessionPollSec` | `6` | how often session status is read (seconds) |
| `hotkeys` | `true` | Ctrl+Opt+1 … 3 for Focus, Focus 2 and the last grid; with `workerNames` also Ctrl+Opt+4 (peek) and Ctrl+Opt+5 (parking lot by hand) |
| `glow` | `true` | colored frames around windows that are done or waiting |
| `glowColor` | `"#5ECFA0"` | frame color for "done" (mint) |
| `glowColorWaiting` | `"#E8B25C"` | frame color for "waiting for you" (amber) |
| `labels` | `true` | whether labels show at first start; after that, your choice in the menu is used |
| `leadNames` | `[]` | own names (`claude --name …` or `/rename`) of sessions that lead other sessions, written exactly the same; an automatic topic title does not count (see "Watching" below); a name like "<team> · …" also makes the session the lead of that team; these sessions never park |
| `otherWindowNames` | `[]` | parts of window names where you answer for other sessions (for example `"front desk"`, any case); a `?` status line that only sends you there does not bring this window forward (see "The session's own status line") |
| `askUntilReadNames` | `[]` | parts of names of places outside the terminal where you handle a request (for example `"card page"`, any case); a `?` status line that ends with one of them in brackets, like `(card page)`, shows WAITING FOR YOU only until you have had its tab in front (see "The session's own status line") |
| `resumeCommands` | `{}` | command to restore a session, per Claude configuration folder, as `{"folder": "command"}` with `{id}` standing for the session, for example `{"~/.claude": "my-launcher claude --resume {id}"}`. A folder without an entry uses `CLAUDE_CONFIG_DIR='<folder>' claude --resume <id>`. A command without `{id}` is ignored |
| `workerNames` | `[]` | parts of session names (any case) that mark worker windows for the parking lot, for example `"Worker · "`; without them there is no parking lot |
| `plannedFile` | none | path of a JSON file that another program writes, starting with `/` or `~`; it adds the Planned area, without it there is none |
| `newSession` | Home, Claude Code | places and programs for new sessions from the +, the role texts of a new team (`team`), see "New sessions from the +" and "Teams, project cards and renaming" |

The menu remembers your choice of screen, layout, grid, control room screens, excluded windows and labels, and
also which windows you sent to the parking lot or took off it by hand.
Lookout also remembers which sessions you parked or pinned in the Inbox, the titles you gave to sessions, your project
cards and your team presets, across restarts. A session that comes back with its id (`claude --resume`) gets its title
back.
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
- It keeps the list of open Claude sessions in `~/Library/Application Support/Lookout/open-sessions.json`.
  The file holds the session id, the folder and the session name, and no conversation text.
- It reads the JSON file named by `plannedFile`, if you set one, and shows its items in the Planned area. It never
  writes to that file. Each time it updates its lists on screen, it checks when the file last changed and reads it
  again if it changed.
- Only when you choose to restore sessions, it writes one launch configuration per session into
  `~/.warp/launch_configurations`, in files whose names start with `lookout-restore-`. It opens each one through
  `open warp://launch/...` and removes it about a minute later.
- Only when you start a new session from the + or from *New session* in the menu bar menu, with a favorite or a free
  choice: Lookout writes a start script, readable only by you, into your temporary folder. It holds the folder and the
  command with the task. The script removes itself when it runs. Lookout also writes a launch configuration into
  `~/.warp/launch_configurations`, in a file whose name starts with `lookout-new-`, and opens it through `open
  warp://launch/...`. It removes both files about a minute later, at once if Warp could not be asked. The launch
  configuration only runs the start script. The task is not in it. The log names the favorite that was started, never
  the task.
- When you start a team, Lookout writes one start script and one launch configuration per window, exactly as for a new
  session from the +; the key account's window starts 3 seconds after the lead's. The log names the team and the roles
  with their place, program, model and effort, never the task. When a session starts with a name, that is a window of a
  team or a session on a card, its start script also sets the environment variable `CLAUDE_SESSION_NAME` to that name.
- Only for a program with `modelsCommand`: once a day Lookout runs that command in the background, with no shell, one
  program after the other and each for at most 30 seconds. It writes the answers with the day to
  `~/Library/Application Support/Lookout/models.json`. The log notes per program how many models it named, that it
  answered nothing usable, or that it could not be asked (failed, could not start, or took too long). Lookout itself
  runs no AI model for this.
- Only when you choose "Empty window" under the + (in the Inbox or under *New session* in the menu bar menu), or when
  Lookout is busy at the very moment you click the +: for Warp it runs `open warp://action/new_window`. For Ghostty it
  runs `osascript` to tell Ghostty to open a window. The first time, macOS asks whether Lookout may control Ghostty.
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

- `?` shows **WAITING FOR YOU**, like an open question, also when only background commands are left or only
  background agents the session started are still running. Claude Code reports such a session as busy; Lookout
  counts it as waiting when nothing came in after the answer. If the
  text in brackets only sends you to a window named in `otherWindowNames` ("approve the plan in the front desk",
  without "here" or "this window"), it counts as `◌`: you answer there, not here. In that window itself it stays `?`.
- If a `?` line ends with the name of a place outside the terminal in brackets, for example `(card page)`, and
  that name is in `askUntilReadNames`, the window shows **WAITING FOR YOU** only until you have had its tab in
  front after the answer. After that the line counts like `◌`: the window shows **WATCHING** again if something
  wakes it on its own, and a worker window goes back to the parking lot. Lead sessions are the exception, see below. For all other sessions, a `?` line without such an ending, or one
  that says "here", stays **WAITING FOR YOU**, read or not.
- `◌` shows **WATCHING** only if something will wake the session on its own (a resting background command,
  another session it talks to, a timer); without that it stays **DONE**, because nothing would continue it.
- `✓` shows **DONE**, even if the answer says "nothing needed".
- For a lead session (named in `leadNames`), `✓` and `?` count like `◌` once you have had its tab in front after
  the answer. The lead then shows **WATCHING** again, but only while something wakes it on its own: a resting
  background command that is not a server, another running session it talks to, or its own timer that is due;
  otherwise it stays DONE. This holds for `?` whether or not the line says "here". A lead should ask its
  questions in a dialog (Claude Code's question dialog, AskUserQuestion). An open dialog is a real open
  question in the session status and keeps showing **WAITING FOR YOU** until you answer it. A question
  written as plain text at the end of a reply is easy to miss. Worker sessions keep showing DONE after `✓`
  so you see which windows can be closed.
- A lead whose status line says `◌` shows WATCHING once you have had its tab in front after the answer, even
  when nothing wakes it on its own. Before you have seen it, it shows DONE like the others. This holds only for
  `◌` (and for a `?` line that already counts as `◌`). A read `✓` from a lead still needs something that wakes it.
  The price: if the lead forgets what it said it would do, Lookout no longer points it out.
- Only the last non-empty line counts, and only for the turn it was written in. The sign decides, not the words, so
  any language works. The text in brackets goes on the label.

To have your sessions write the line, copy the hook template `hooks/lookout_line.py`. It is Python, standard library
only. It gives every interactive Claude Code session the instruction as `additionalContext`: at every session start
(new session, resume, `/clear` and `/compact`) and with every prompt you send.

It skips background runs (`claude -p` and Agent SDK runs), because scripts read their output and an extra line can
break them. A line in `CLAUDE.md` would reach those runs too, so I use a hook.

The instruction is the constant `LOOKOUT_LINE` at the top of the file. It is the English version of the instruction my
own hook gives my sessions, without my name and my window names. My own hook does more: it also names my sessions, and
most of my sessions get the instruction in German. The template contains only the Lookout part.

It runs with Apple's `/usr/bin/python3` (Python 3.9 or newer). That comes with the Xcode Command Line Tools, and
those are needed to build Lookout anyway. No extra packages.

Setup, from the `lookout` folder:

```sh
mkdir -p ~/.claude/hooks && cp hooks/lookout_line.py ~/.claude/hooks/
```

Then add this to `~/.claude/settings.json`. If the file already has a `"hooks"` block, add the two entries to it:

```json
{
  "hooks": {
    "SessionStart": [
      { "hooks": [{ "type": "command", "command": "/usr/bin/python3 ~/.claude/hooks/lookout_line.py" }] }
    ],
    "UserPromptSubmit": [
      { "hooks": [{ "type": "command", "command": "/usr/bin/python3 ~/.claude/hooks/lookout_line.py" }] }
    ]
  }
}
```

Every Claude Code configuration folder (`CLAUDE_CONFIG_DIR`, for example `~/.claude-work`) has its own
`settings.json`. Add the same block there too. After a Lookout update (`git pull`), copy the file again.

You can change the text of `LOOKOUT_LINE`, for example translate it. Keep each of the three signs followed by a space
and `Lookout:`. The sign decides, not the words.

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

## Planned (optional)

The setting `plannedFile` in `~/.config/lookout.json` names a JSON file that another program writes, for example a
scheduler of yours. The path starts with `/` or `~`. Any other path is ignored. Without the setting there is no
Planned tile and no Planned area.

Lookout only reads the file, it never writes it. Each time Lookout updates its lists on screen (the Inbox, or the side
lists of the other layouts), it checks when the file last changed, and reads it again if it changed. A missing or
unreadable file shows an empty Planned area, and Lookout tries again at the next update. Times are shown in the Mac's
time zone.

The file holds `{"items": [...]}` or just the list `[...]`. Each item has a `title`, and either `at` or `date`. The
other keys are optional.

- `title`: the title of the item.
- `at`: a time with its offset (RFC 3339, like "2026-11-12T09:30:00+01:00"). It makes the item a start: a session that
  the program writing the file opens by itself at that time. Lookout itself does not start it. It only shows the item.
- `date`: a day (YYYY-MM-DD). It makes the item a task due that day, started by hand.
- `project`: shown below the title.
- `detail`: one line, for example how the start runs.
- `problem`: for a start that did not happen, why.
- `deadline`: a day (YYYY-MM-DD) for a hard deadline.
- `since_days`: a whole number, for how many days the task has been due.

An example with one start and one task:

```json
{
  "items": [
    {"title": "Send the weekly report", "project": "reports", "at": "2026-11-12T09:30:00+01:00", "detail": "claude · sonnet"},
    {"title": "Renew the domain", "date": "2026-11-10", "deadline": "2026-11-13", "since_days": 2}
  ]
}
```

Keys Lookout does not know are ignored. Broken items are skipped, and a file that is not valid JSON counts as empty.
Runs of spaces and line breaks in the texts become one space.

The Planned tile shows how many items stand in the current week from today on. Its first line is the next start, like
"▶ today 10:00" and its title, or "tomorrow", or a date. If a start did not happen, the tile is amber and its first
line reads "! Did not start:" and the title instead. Its second line counts the items of today and tomorrow, like "2
today · 1 tomorrow".

The Planned area shows a week strip at the top: the title of the week (like "Week 46 · November 9 to 15"), an arrow ‹
for the week before and › for the week after, and the seven days from Monday to Sunday. Today's number is mint, and
the chosen day is highlighted. A click on a day shows that day. When you open Planned, today is chosen, and it stays
today after midnight. In another week the Monday is chosen. Back in the current week, today is chosen.

Each day in the strip has a dot per card it lists, at most four. Amber is for a start that did not happen or a hard
deadline, mint for a start, and an open ring for a task. Amber dots come first.

Below the strip stands the chosen day ("Today · Tue Nov 10", "Tomorrow · Wed Nov 11", or a date like "Thu Nov 12"),
then its cards. Starts come first, by their time, then tasks, the one due longest first. Then, for items listed on
another day whose hard deadline is this day, comes a card like "Hard deadline Fri Nov 13, listed on Thu Nov 12". With
nothing on the day, you see "Nothing planned that day."

A task due before today, and a start whose time has passed, stand on today. A start stays there until the program that
writes the file removes it. If that program gives it a `problem`, it still stays on today, as a card with the amber !
mark that reads "Did not start:" and the problem. Only the round mark is amber, not the whole card.

A card shows the title, the project below it, and for a start its time at the right. Its mark is ▶ for a start, ! for
a start that did not happen and for a hard deadline (both amber), and an open ring for a task. Its line of text reads
"Starts by itself", followed by " · " and the detail if there is one. For a start that did not happen it reads "Did
not start:" and the problem. For a task it reads "Deadline Fri Nov 13", "Due for 5 days", "Due since yesterday", "Due
from today" or "Due from Thu Nov 12". Planned cards open no session and have no right-click menu.

## New sessions from the +

A click on the + in the head of the Inbox opens a menu right below it. It lists three favorites, numbered ①, ② and ③.
Each shows place, program, model and effort, separated by " · ", for example "① Home · Claude Code · opus · max".
Below them stand "Choose freely", "New team …", "Empty window" and, after a separator line, "Change favorites". "Empty
window" does
what the + did before. If Lookout is busy at the very moment you click the +, it opens such an empty window right away
instead of the menu. The same menu is under *New session* in the menu bar menu. Without any AI session there is no
Inbox and no +, so a first session can start from the menu bar.

"Choose freely" opens submenus: the places, in each the programs, in each the program's models, in each the program's
efforts. A level the program has no list for is left out. A program without models goes straight to its efforts. One
without models and efforts starts from its own row. The last click opens the same "New task" window and starts exactly
that choice. The favorites stay as they are.

Choosing a favorite opens a small window "New task" that shows the favorite and has a text field. The text wraps in
the field. Above the task stands a field "Card": leave it empty for none, pick a card from its list or type a new name.
The session then starts on that card, see "Teams, project cards and renaming". Enter or the button "Start" starts. Esc
or "Cancel" cancels, and the app that was in front before gets the
front back. An empty field starts the session without a task. Copy, paste, cut, select all and undo work in the field
with ⌘C, ⌘V, ⌘X, ⌘A and ⌘Z. The task becomes one line: line breaks and runs of spaces become one space.

After Enter, a new Warp window opens in the place's folder, starts the program with the favorite's model and effort,
and gives the task to the program as its first message. Lookout arranges that window like every other. The window
closes when the session ends normally, and stays open with the error if the start fails.

"Change favorites" has a submenu for each favorite, and in it Place, Program, Model and Effort. Each lists its choices
with a check mark on the current one. Places, programs and efforts come from the settings. Models come from the
program's current model list (see below), or from the settings if there is none. One click saves at once. There is no
save button. Choosing another program keeps the model and the effort if the new program offers them. Otherwise the
favorite gets the program's first model and first effort. Model and Effort cannot be opened while the favorite's
program is missing from the settings. Lookout keeps the three favorites in its own memory (the macOS user defaults),
not in the settings file, so they stay through a restart. A slot without a stored favorite gets the first place, the
first program and that program's first model and effort.

A favorite whose place or program is no longer in the settings (for example because it was renamed), or whose model or
effort the program no longer offers, stands greyed out with "not available". It can still be changed under "Change
favorites". A choice that no longer fits the settings never starts, for example because the model lists changed
meanwhile. Usually no "New task" window opens at all. If the list changed while the window was open, nothing starts
after Enter and the typed task is gone.

Places and programs come from the setting `newSession` in `~/.config/lookout.json`. Lookout reads it once at start,
like every other setting. For example:

```json
{
  "newSession": {
    "places": [
      {"name": "Work", "folder": "~/work"},
      {"name": "Home", "folder": "~"}
    ],
    "programs": [
      {"name": "Claude Code", "models": ["opus", "sonnet"], "efforts": ["max", "high"],
       "command": ["claude", "--model", "{model}", "--effort", "{effort}", "--", "{task}"]},
      {"name": "Codex", "models": ["gpt-5.5"], "efforts": ["high", "medium"],
       "command": ["codex", "--model", "{model}", "-c", "model_reasoning_effort={effort}", "--", "{task}"],
       "modelsCommand": ["/usr/local/bin/list-models", "codex"]}
    ]
  }
}
```

The path `/usr/local/bin/list-models` stands for a small script of your own that prints the models.

The rules:

- `places`: each place has a `name` and a `folder`. The folder starts with `/`, or is `~` or starts with `~/` (your
  home folder). A place with any other folder is ignored, and `~name` is not read as a home folder. Any other text
  value of a place, for example `"profile": "work"`, stands for `{profile}` in a command.
- `programs`: each program has a `name`, `models` (a list), `efforts` (a list) and `command`. The command is a list of
  words, not a shell line. Each word is kept exactly as written, an empty word too.
- In the words, `{model}` and `{effort}` stand for the favorite's model and effort, `{task}` for the task, and `{key}`
  for a place's value of that name. A placeholder can stand inside a word, for example
  `model_reasoning_effort={effort}`. `{model}`, `{effort}` and `{task}` always come from the favorite and the task
  window, so a place value with one of these names does not count.
- After filling in, every word is put in single quotes for the shell as a whole, so nothing in a value or in the task
  is ever read by the shell.
- `{task}` must appear exactly once in the command, as a word of its own, right after a word `--`. Otherwise the
  program is ignored. The `--` keeps a task that starts with a dash, like `--version`, from being read as an option.
  Without a task, `--` and `{task}` are left out.
- `{name}` stands for the session's name. It must be a word of its own, right after an option (a word that starts with
  "-", but not `--`), and it may appear at most once; otherwise the program is ignored. When a session starts without a
  name, the option and `{name}` are left out. When it starts with a name, the start script also sets the environment
  variable `CLAUDE_SESSION_NAME` to it, so that a hook that names sessions by their first message can keep it. Only
  programs with `{name}` can be the lead or the key account of a team, or start a session on a card.
- A command that uses `{model}` or `{effort}` needs a non-empty `models` or `efforts` list. Otherwise the program is
  ignored.
- If two places or two programs have the same name, the first valid one counts and later ones are ignored. An entry
  ignored for one of the reasons above does not take the name.

A program can also have `modelsCommand`. It is a word list like `command`, run as it is, with no shell and no
placeholders filled in. It answers with the program's current models, one per line. A line may carry, after a tab, the
name the menu shows. For example, the line "opus", a tab, "Opus 5.5" makes the menu, the task window and the
favorites' text show "Opus 5.5". The command gets `opus`, and a favorite keeps `opus`. Lookout runs these commands
once a day, in the background, one after the other, each for at most 30 seconds. It keeps the lists with the day in
`~/Library/Application Support/Lookout/models.json`. A restart on the same day asks nothing again, unless a program
has no list there yet. A command that fails, answers nothing or takes longer keeps the last list. Blank lines and
repeated models are left out, and a program keeps at most 40 models. `models` stays the list until the first answer
and whenever there is none. Running these commands costs whatever they do. Lookout itself runs no AI model for them.

Without the setting, or with nothing usable in it, there is one place, "Home" (your home folder), and one program,
"Claude Code". Its command is `["claude", "--name", "{name}", "--model", "{model}", "--effort", "{effort}", "--",
"{task}"]`, its models
are opus, sonnet and haiku, and its efforts are max, xhigh, high, medium and low.

## Teams, project cards and renaming

A team is a group of AI windows whose names start with the team's name and " · " (space, middle dot, space), for example
"Shop · Team lead", "Shop · Key account" and "Shop · checkout page". Lookout knows a team only by these names. A team
exists while a window of it is open and one of these windows founds it: a window named "<team> · Team lead" (or
"Team-Lead"), a window named "<team> · Key account", or a window whose own name is in the setting `leadNames`. Names
compare in any case, a hyphen counts as a space and runs of spaces count as one. "Team lead" (or "Team-Lead") is the
lead, "Key account" is the key account, and every other name is a worker. A `leadNames` entry is a lead too. The name of
a window is the title you gave it in Lookout, else the session's own name, else the name from the status file of Claude
Code. If two windows of a team have the same name, only the newest keeps its role; older ones stand under "To close" (a
lead that handed over keeps its name, the successor takes it). Lookout does not start workers: the lead starts them
itself, named "<team> · <step>", as its role text says.

The menu of the + has "New team …" between "Choose freely" and "Empty window", and *New session* in the menu bar menu
has it too. It is greyed out with "no program with {name}" while no program in the settings has `{name}` in its command.
"New team …" opens a window "New team" with these fields:

- "Team name", with the example "e.g. Shop".
- "Team lead": a button that opens the same cascade as "Choose freely" (place, program, model, effort); the place chosen
  here is the team's place.
- "Key account": a tick box and a button with program, model and effort. It starts in the lead's place.
- "Workers": a button with program, model and effort.
- "Task": a text field.

The buttons are "Start" and "Cancel" (Esc). Lead and key account can only use programs with `{name}`. Workers can use
any program: the choice starts nothing, it goes into the lead's text. The team name must not be empty, must not contain
"·", has at most 40 characters, and must not be the name of a team that is open or was just started (any case). A role
whose choice no longer fits the settings also keeps the window open. The window says what is wrong and keeps the task.
The choices in the window count only for this team.

On "Start" the lead starts in a new Warp window named "<team> · Team lead". With the tick, the key account follows 3
seconds later in a window named "<team> · Key account", because Warp opened only one of two windows asked for at the
same moment. Each window gets its role text, then a space, then the task, as its first message.

The role texts are the setting `newSession.team` in `~/.config/lookout.json`: `lead` for the lead of a team with a key
account, `leadAlone` for a lead without one, and `keyAccount`. A text that is missing or empty keeps the built-in
English text. These placeholders work in the texts: `{team}` (the team's name), `{lead}` and `{keyAccount}` (the names
of the two windows), `{worker}` (the workers' choice as "program · model · effort", the model with the name from the
daily model list), `{workerProgram}`, `{workerModel}` and `{workerEffort}`. For example:

```json
{
  "newSession": {
    "team": {
      "lead": "You lead the team {team}. Workers start with {worker}. Questions go to \"{keyAccount}\". The task:",
      "leadAlone": "You lead the team {team}. Workers start with {worker}. The task:",
      "keyAccount": "You are the key account of the team {team}. The team's task:"
    }
  }
}
```

Under "Change favorites › Team" you set the presets of the "New team" window: the place and, for "Team lead: …", "Key
account: …" and "Workers: …", the program, model and effort. One click saves at once. Without a stored preset, lead and
key account get the first place and the first program with `{name}`, with its first model and effort, workers get the
first program with its first model and effort, and the key account tick is on.

Each team has a tile on the overview of the Inbox, right below Waiting, two tiles side by side. The tile shows the
team's name and how many windows it has. Its first line is "? <name> asks you" while one of its windows asks, else the
state signs of lead and key account ("● Lead · ✓ Key account", or "no lead"). Its second line says how many workers run.
The tile is amber while one of its windows asks, and it goes away when no window of the team is open any more.

A click on the tile opens the team's area with the groups "Team lead", "Key account", "Workers" and "To close". Each
card there shows the session's model under its name (for example "Opus 5.5", Claude Code only). If the team is gone
while its area is open, the Inbox goes back to the overview. The windows of a team show only in its area: not under
Waiting, Running or Parked, and they do not count there. Pinned ones stay in Pinned as well.

A project card gathers every window whose name starts with "<project> · ", like a team but without roles. You make one
in one of two ways. Right-click a card in Waiting, Running or Parked whose name starts with "<project> · " and that is
in no team, and choose "Own card for “<project>”". Or use the field "Card" in the "New task" window of the +: pick a
card from its list or type a new name. The session then starts named "<card> · <first words of the task>", with at most
40 characters of the task, a longer first word cut, and a "·" in the card name turned into a space; without a task it is
named "<card> · new". The card is kept. This works only with a program that has `{name}`; with another program the
session starts without a card and the log says so.

The windows of a card leave Waiting, Running and Parked, like those of a team. Its tile stands with the team tiles and
shows its name and how many windows it has. Its first line is "? <name> asks you" while one window asks, else how many
are done and, if any, how many are stopped ("1 done · 1 stopped"). Its second line says how many work. The tile is amber
while one window asks. A click on it opens the card's area, grouped by state: "Questions for you", "Stopped", "Working"
and "Done". Windows with the same name all stay. "Dissolve the card" in the right-click menu of each of its windows in
its area ends the card. A card stays until you dissolve it, also across restarts. A card whose project is also a team
shows as that team.

A right-click on any card in the Inbox, also in the area of a team or a card, offers "Rename …". A small window "Rename"
shows the session's current full name, for example "Shop · checkout page". "Save" keeps the title, "Cancel" or Esc
cancels. An empty field, or the session's own name unchanged, gives the session its own name back. The title shows on
the card, on the label and in the menu instead of the session's own name. It also decides the team or card: "<project> ·
…" puts the window in that project's team or card if there is one, and a title without " · " leaves it on its own. The
session itself keeps its name. Lookout does not rename the session, its tab or what `/resume` shows, and to find a
window it still uses the session's own names.

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
- New sessions from the + open only in Warp. The task becomes one line. There are three favorites, no more. A program
  can only be used if it takes `--` as the end of its options.
- Teams and project cards know their windows only by name; a window that gets another name, for example from a hook that
  names sessions, leaves its team or card. Lookout starts only the lead and the key account, the lead starts the workers
  with its own tools, and the workers' choice only goes into the lead's text. Team windows and sessions on a card open
  only in Warp, like every new session from the +. A project card with no window open shows no tile; it stays in the
  list of the field "Card" and can only be dissolved from one of its windows. "Rename …" changes only what Lookout
  shows. STOPPED is recognized for Claude Code and Codex only.

## Kurz auf Deutsch

Lookout ist eine kleine Menüleisten-App, die ich mit KI-Hilfe für meinen eigenen Schreibtisch gebaut habe: Sie ordnet
die Terminalfenster deiner KI-Sessions (Claude Code, Codex, Grok, Kimi in Warp oder Ghostty) und zeigt ruhig, wer
arbeitet, wer fertig ist und wer auf dich wartet. Fertig ist mintgrün umrahmt, bernsteinfarben heisst: die Session
braucht deine Freigabe oder deine Auswahl in einem Fragefenster. Endet eine Antwort mit einer Lookout-Zeile
(`◌` / `?` / `✓ Lookout: …`, siehe oben), gilt, was die Session selbst sagt. Seit Version 4 ist Lookout in Rust
neu geschrieben. Mit der Einstellung `workerNames` warten die Fenster von Worker-Sessions hinter deiner aktiven
Session (Parkplatz); Ctrl+Opt+4 zeigt sie.
Der neue «Eingang» ist ein viertes Layout: Sein Fenster steht direkt neben dem aktiven Terminal und hat eine Übersicht
mit einer Kachel pro Bereich, und Lookout startet auf ihr; ein Klick auf die Kachel öffnet den Bereich, und zurück
geht es mit «‹ Übersicht» oder indem du mit zwei Fingern seitwärts wischst. Mit der Einstellung `plannedFile` gibt es
zusätzlich den Bereich «Geplant». Das «+» im Eingang öffnet ein Menü mit drei Favoriten (Ort, Programm, Modell,
Effort), «Frei wählen», «Leeres Fenster» und «Favoriten ändern». Ein Favorit oder eine freie Wahl fragt nach einem
Auftrag und startet damit eine neue Session in einem eigenen Warp-Fenster; Orte und Programme kommen aus der
Einstellung `newSession`, die Modell-Listen auf Wunsch einmal am Tag über einen eigenen Befehl je Programm
(`modelsCommand`). Dasselbe Menü steht in der Menüleiste unter «Neue Session». Mit «Parken» im Rechtsklick-Menü einer
Karte parkst du eine Session, der du nicht folgen willst; im Bereich «Geparkt» holst du sie mit «Zurückholen» wieder.
Mit «Anheften» im Rechtsklick-Menü einer Karte stellst du die Session in die Gruppe «Angeheftet» ganz oben im Bereich
«Wartet»; dort bleibt sie, bis du «Lösen» wählst. «Neues Team …» im «+» startet ein Fenster «Team-Lead» und auf Wunsch
ein Fenster «Key account», jedes mit einem Rollentext vor dem Auftrag. Jedes Team und jede Projekt-Karte hat in der
Übersicht eine eigene Kachel, und ihre Fenster stehen nur noch dort. Mit «Umbenennen …» im Rechtsklick-Menü gibst du
einer Session in Lookout einen eigenen Titel. Ein mit Esc abgebrochener Zug zeigt ⏸ ANGEHALTEN in Blaugrau. Nach einem
Absturz, einem Herunterfahren oder wenn Warp beendet
wurde, zeigt das Menü «Sessions wiederherstellen»; jede Claude-Code-Session öffnet sich dann in einem eigenen
Warp-Fenster und startet `claude --resume`.
Installation wie oben (`git clone`, `./build.sh`,
Bedienungshilfen freigeben). Die Sprache folgt dem Mac; fest auf Deutsch mit `{"language": "de"}` in
`~/.config/lookout.json`. Die Schilder zeigen Text aus deinen Sessions: vor dem Bildschirmteilen unter
*Mehr > Schilder anzeigen* ausschalten.

## License

MIT, see [LICENSE](LICENSE). Lookout is an independent project and not affiliated with Anthropic, OpenAI,
xAI, Moonshot AI, Warp or Ghostty. All product names are trademarks of their respective owners.

Kevin Theo ([LinkedIn](https://www.linkedin.com/in/kevintheo) · [kevintheo.com](https://kevintheo.com))

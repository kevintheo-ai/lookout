//! Processes and small OS services, read directly instead of starting `ps` every tick (plan: "Prozesse direkt im
//! Programm"). Swift: Mapper.swift `processCwd`, `processStartDate`, `processArguments`, `engineProcesses`,
//! `writeOSCTitle`; Watcher.swift `processAlive`, `nettopUsage`, `listeningPids`; Common.swift `screenLocked`;
//! Main.swift `isLaptop`. Callable from any thread.

use std::collections::{BTreeMap, BTreeSet};

use lookout_core::model::{EngineProc, Pid, PsRow, Time};

/// Every process (`sysctl(CTL_KERN, KERN_PROC, KERN_PROC_ALL)`): pid, ppid, controlling terminal as "ttysNNN" (like
/// `ps -o tty=`) or "??" without one, CPU time user + system in seconds (`proc_pidinfo(PROC_PIDTASKINFO)`, 0 when not
/// readable), `comm` = the executable's path from `proc_pidpath`, never a command line (core `engine_of` takes its last
/// path part); not readable (another user's process) -> the short name `p_comm`. `None` = the list could not be read:
/// the caller keeps the last one (review 2026-09-25 finding 10: an empty list dropped every window match).
pub fn ps_rows() -> Option<Vec<PsRow>> {
    todo!()
}

/// The AI CLIs with a terminal and all rows (Swift `engineProcesses`): `lookout_core::mapping::process_list` over
/// `ps_rows`, working folder per `process_cwd` ("" when unreadable). `terminal_pids`: running Warp/Ghostty processes.
pub fn engine_processes(terminal_pids: &BTreeSet<Pid>) -> Option<(Vec<EngineProc>, Vec<PsRow>)> {
    let _ = terminal_pids;
    todo!()
}

/// Working folder (libproc `proc_pidinfo(PROC_PIDVNODEPATHINFO)`, no subprocess).
pub fn process_cwd(pid: Pid) -> Option<String> {
    let _ = pid;
    todo!()
}

/// Process start (`sysctl KERN_PROC_PID`, `kp_proc.p_un.__p_starttime`), like `ps -o lstart`, also for root processes
/// (libproc gives nothing there, measured 27.09.). `None` = no such process (sysctl answers success with length 0).
pub fn process_start(pid: Pid) -> Option<Time> {
    let _ = pid;
    todo!()
}

/// Start arguments as started (`sysctl KERN_PROCARGS2`: argc, the program path, padding, then the arguments, each ended
/// by NUL). `None` = not readable or fewer arguments than argc.
pub fn process_arguments(pid: Pid) -> Option<Vec<String>> {
    let _ = pid;
    todo!()
}

/// Does the process live? `kill(pid, 0)`; EPERM (another user's process, root) lives too. pid 0 and negative pids mean
/// process groups, never a session: false.
pub fn process_alive(pid: Pid) -> bool {
    let _ = pid;
    todo!()
}

/// Writes `ESC ] 0 ; title BEL` to `/dev/<tty>` (`O_WRONLY | O_NONBLOCK`): the terminal sets the tab title, the running
/// program never sees it. The core already filtered control characters out of `title`. false = not written.
pub fn write_osc_title(tty: &str, title: &str) -> bool {
    let _ = (tty, title);
    todo!()
}

/// Screen locked (`CGSessionCopyCurrentDictionary`, key `CGSSessionScreenIsLocked`)? Then AX still lists the terminal
/// windows but without readable titles, a ping would never see its marker (live 2026-09-25 19:35).
pub fn screen_locked() -> bool {
    todo!()
}

/// Network bytes per process: `/usr/bin/nettop -P -x -n -L 1 -J bytes_in,bytes_out` via core `run_command` (time limit
/// 3 s, log lines to `crate::log`), parsed by core `parse_nettop`. `None` = the call failed. `-n`: with name resolution
/// a call took 5 s instead of 0.04 s (measured 2026-09-26).
pub fn nettop_usage() -> Option<BTreeMap<Pid, u64>> {
    todo!()
}

/// Which of these processes listen on TCP (servers)? `/usr/sbin/lsof -nP -a -p <pids joined by ","> -iTCP
/// -sTCP:LISTEN -Fp`, exit codes 0 and 1 accepted (1 = nothing listens or a process is gone), time limit 3 s, parsed
/// by core `parse_lsof_pids`. No pids = empty set without a call. `None` = the call failed.
pub fn listening_pids(pids: &[Pid]) -> Option<BTreeSet<Pid>> {
    let _ = pids;
    todo!()
}

/// A laptop (internal battery, IOKit `IOPSCopyPowerSourcesInfo`): then the built-in screen is called MacBook, else iMac.
/// Unknown: true (MacBook, as before).
pub fn is_laptop() -> bool {
    todo!()
}

/// The user's full name (Foundation `NSFullUserName`), for the greeting the labels strip.
pub fn full_user_name() -> String {
    todo!()
}

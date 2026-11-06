//! Processes and small OS services, read directly instead of starting `ps` every tick (plan: "Prozesse direkt im
//! Programm"). Swift: Mapper.swift `processCwd`, `processStartDate`, `processArguments`, `engineProcesses`,
//! `writeOSCTitle`; Watcher.swift `processAlive`, `nettopUsage`, `listeningPids`; Common.swift `screenLocked`.
//! Callable from any thread.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{CStr, c_char, c_int};
use std::mem::{offset_of, size_of};
use std::ptr;
use std::sync::{Mutex, OnceLock};

use lookout_core::mapping::process_list;
use lookout_core::model::{EngineProc, Pid, PsRow, Time};
use lookout_core::status::{parse_lsof_pids, parse_nettop, run_command};

/// Every process (`sysctl(CTL_KERN, KERN_PROC, KERN_PROC_ALL)`): pid, ppid, controlling terminal as "ttysNNN" (like
/// `ps -o tty=`) or "??" without one, CPU time user + system in seconds (`proc_pidinfo(PROC_PIDTASKINFO)`, 0 when not
/// readable). `comm` exactly like `ps -o comm=`: the program AS STARTED, i.e. argv[0] from `sysctl KERN_PROCARGS2` (a
/// symlink stays unresolved, a bare name stays bare), never the further arguments. ps prints argv[0], not the exec path
/// in front of it (measured 2026-11-06 over 1110 own processes: argv[0] = ps comm for all, the exec path for 946 only;
/// `sleep` started from a shell is `sleep` in ps, its exec path `/bin/sleep`). A program that rewrites its argv[0] shows
/// the new text, in ps as here (node's process title: `npm exec @playwright/mcp@0.0.79 …`). NOT `proc_pidpath`: it
/// resolves symlinks, and Claude runs as `~/.local/bin/claude` -> `~/.local/share/claude/versions/2.1.319`, whose last
/// path part is no engine name (core `engine_of`; measured 2026-11-06: ps comm `/Users/kevin/.local/bin/claude`,
/// proc_pidpath `.../versions/2.1.319`). Not readable (another user's process) -> the short name `kp_proc.p_comm`.
/// pid 0 (kernel_task, ppid 0) is in the list, ps leaves it out. ~8 ms for 1440 processes, the first call ~25 ms
/// (devname, see `tty_name`; measured 2026-11-06, `/bin/ps` took 120 ms). `None` = the list could not be read: the
/// caller keeps the last one (review 2026-09-25 finding 10: an empty list dropped every window match).
pub fn ps_rows() -> Option<Vec<PsRow>> {
    let procs = process_table()?;
    // One buffer for all argument areas: it grows to the largest and stays for the rest of the list.
    let mut args = vec![0u8; 64 << 10];
    Some(
        procs
            .iter()
            .map(|k| PsRow {
                pid: k.pid,
                ppid: k.ppid,
                tty: tty_name(k.tdev),
                cpu: cpu_seconds(k.pid),
                // A zombie is "<defunct>" in ps: else a dead shell child of Claude would count as a background job
                // (core `background_tasks` takes shells by name; measured 2026-11-06, 5 zombies, ps "<defunct>").
                comm: if k.stat == SZOMB {
                    "<defunct>".into()
                } else {
                    procargs(k.pid, &mut args)
                        .and_then(program_name)
                        .unwrap_or_else(|| c_string(&k.comm))
                },
            })
            .collect(),
    )
}

/// The AI CLIs with a terminal and all rows (Swift `engineProcesses`): `lookout_core::mapping::process_list` over
/// `ps_rows`, working folder per `process_cwd` ("" when unreadable). `terminal_pids`: running Warp/Ghostty processes.
pub fn engine_processes(terminal_pids: &BTreeSet<Pid>) -> Option<(Vec<EngineProc>, Vec<PsRow>)> {
    let rows = ps_rows()?;
    let procs = process_list(&rows, terminal_pids, |pid| {
        process_cwd(pid).unwrap_or_default()
    });
    Some((procs, rows))
}

/// Working folder (libproc `proc_pidinfo(PROC_PIDVNODEPATHINFO)`, no subprocess).
pub fn process_cwd(pid: Pid) -> Option<String> {
    // SAFETY: integers and byte arrays only; all zero is a valid value.
    let mut info: libc::proc_vnodepathinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_vnodepathinfo>() as c_int;
    // SAFETY: `info` is writable for `size` bytes; proc_pidinfo writes at most that many.
    let got = unsafe {
        libc::proc_pidinfo(
            pid,
            libc::PROC_PIDVNODEPATHINFO,
            0,
            (&raw mut info).cast(),
            size,
        )
    };
    (got == size).then(|| c_string(info.pvi_cdir.vip_path.as_flattened()))
}

/// Process start (`sysctl KERN_PROC_PID`, `kp_proc.p_un.__p_starttime`), like `ps -o lstart`, also for root processes
/// (libproc gives nothing there, measured 27.09.). `None` = no such process (sysctl answers success with length 0).
pub fn process_start(pid: Pid) -> Option<Time> {
    let mut info = KinfoProc::ZERO;
    let mut size = size_of::<KinfoProc>();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, pid];
    // SAFETY: `mib` holds the 4 names passed; `info` is writable for `size` bytes, the kernel writes at most that many
    // and leaves the count in `size`.
    let r = unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            4,
            (&raw mut info).cast(),
            &mut size,
            ptr::null_mut(),
            0,
        )
    };
    (r == 0 && size == size_of::<KinfoProc>())
        .then(|| info.start_sec as f64 + f64::from(info.start_usec) / 1_000_000.0)
}

/// Start arguments as started (`sysctl KERN_PROCARGS2`: argc, the program path, padding, then the arguments, each ended
/// by NUL). `None` = not readable or fewer arguments than argc.
pub fn process_arguments(pid: Pid) -> Option<Vec<String>> {
    let mut buf = vec![0u8; 64 << 10];
    procargs(pid, &mut buf).and_then(argument_strings)
}

/// Does the process live? `kill(pid, 0)`; EPERM (another user's process, root) lives too. pid 0 and negative pids mean
/// process groups, never a session: false.
pub fn process_alive(pid: Pid) -> bool {
    // SAFETY: signal 0 sends nothing, it only checks the pid.
    pid > 0
        && (unsafe { libc::kill(pid, 0) } == 0
            || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM))
}

/// Writes `ESC ] 0 ; title BEL` to `/dev/<tty>` (`O_WRONLY | O_NONBLOCK`): the terminal sets the tab title, the running
/// program never sees it. The core already filtered control characters out of `title`. false = not written.
/// `O_NOCTTY` beyond Swift: Lookout has no terminal of its own, and opening one without it may make it Lookout's
/// controlling terminal (its hangup would then end Lookout). std adds `O_CLOEXEC`: other threads start commands.
pub fn write_osc_title(tty: &str, title: &str) -> bool {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY)
        .open(format!("/dev/{tty}"))
        .and_then(|mut f| f.write(format!("\x1b]0;{title}\x07").as_bytes()))
        .is_ok_and(|n| n > 0)
}

/// Screen locked (`CGSessionCopyCurrentDictionary`, key `CGSSessionScreenIsLocked`)? Then AX still lists the terminal
/// windows but without readable titles, a ping would never see its marker (live 2026-09-25 19:35).
pub fn screen_locked() -> bool {
    use objc2_core_foundation::{CFBoolean, CFDictionary, CFString, CFType};
    let Some(session) = objc2_core_graphics::CGSessionCopyCurrentDictionary() else {
        return false;
    };
    // SAFETY: the session dictionary maps CFString keys to CF objects (CGSession.h); the value is checked below. The
    // flags in it are CFBooleans (measured 2026-11-06: kCGSSessionOnConsoleKey, kCGSessionLoginDoneKey).
    let session: &CFDictionary<CFString, CFType> = unsafe { session.cast_unchecked() };
    session
        .get(&CFString::from_static_str("CGSSessionScreenIsLocked"))
        .and_then(|v| v.downcast::<CFBoolean>().ok())
        .is_some_and(|locked| locked.as_bool())
}

/// Network bytes per process: `/usr/bin/nettop -P -x -n -L 1 -J bytes_in,bytes_out` via core `run_command` (time limit
/// 3 s, log lines to `crate::log`), parsed by core `parse_nettop`. `None` = the call failed. `-n`: with name resolution
/// a call took 5 s instead of 0.04 s (measured 2026-09-26).
pub fn nettop_usage() -> Option<BTreeMap<Pid, u64>> {
    let args = ["-P", "-x", "-n", "-L", "1", "-J", "bytes_in,bytes_out"];
    command_text("/usr/bin/nettop", &args, &[0]).map(|text| parse_nettop(&text))
}

/// Which of these processes listen on TCP (servers)? `/usr/sbin/lsof -nP -a -p <pids joined by ","> -iTCP
/// -sTCP:LISTEN -Fp`, exit codes 0 and 1 accepted (1 = nothing listens or a process is gone), time limit 3 s, parsed
/// by core `parse_lsof_pids`. No pids = empty set without a call. `None` = the call failed.
pub fn listening_pids(pids: &[Pid]) -> Option<BTreeSet<Pid>> {
    if pids.is_empty() {
        return Some(BTreeSet::new());
    }
    let list = pids
        .iter()
        .map(Pid::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let args = ["-nP", "-a", "-p", &list, "-iTCP", "-sTCP:LISTEN", "-Fp"];
    command_text("/usr/sbin/lsof", &args, &[0, 1]).map(|text| parse_lsof_pids(&text))
}

/// The user's full name (Foundation `NSFullUserName`), for the greeting the labels strip.
pub fn full_user_name() -> String {
    // A pool of its own: callable from threads that have none.
    objc2::rc::autoreleasepool(|_| objc2_foundation::NSFullUserName().to_string())
}

// ---------- Helpers ----------

/// A command's output as text, inherited environment, time limit 3 s. Lossy UTF-8 where Swift answered nil: nettop
/// names are `p_comm` cut at 15 bytes, which can split a character; only pids and numbers are read.
fn command_text(path: &str, args: &[&str], accept: &[i32]) -> Option<String> {
    run_command(
        path,
        args,
        &BTreeMap::new(),
        &[],
        3.0,
        accept,
        &mut |line| crate::log(line),
    )
    .map(|data| String::from_utf8_lossy(&data).into_owned())
}

/// `p_stat` of a zombie (<sys/proc.h>).
const SZOMB: u8 = 5;

/// The process table (`sysctl KERN_PROC_ALL`).
fn process_table() -> Option<Vec<KinfoProc>> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL];
    // Processes started between asking the size and reading the table: ENOMEM, ask again.
    for _ in 0..5 {
        let mut size = 0;
        // SAFETY: `mib` holds the 3 names passed; without a buffer the kernel only stores the size in `size`.
        let r = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                ptr::null_mut(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        if r != 0 {
            return None;
        }
        let mut procs = vec![KinfoProc::ZERO; size / size_of::<KinfoProc>() + 64];
        let mut size = procs.len() * size_of::<KinfoProc>();
        // SAFETY: as above; `procs` is writable for `size` bytes, the kernel writes whole entries and leaves the count
        // in `size`.
        let r = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                procs.as_mut_ptr().cast(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        if r == 0 {
            procs.truncate(size / size_of::<KinfoProc>());
            return Some(procs);
        }
        if std::io::Error::last_os_error().raw_os_error() != Some(libc::ENOMEM) {
            return None;
        }
    }
    None
}

/// `sysctl KERN_PROCARGS2` of `pid` into `buf`, which grows as needed and is kept by the caller. `None` = not readable
/// (another user's process, gone) or nothing after argc.
fn procargs(pid: Pid, buf: &mut Vec<u8>) -> Option<&[u8]> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    loop {
        let mut size = buf.len();
        // SAFETY: `mib` holds the 3 names passed; `buf` is writable for `size` bytes, the kernel writes at most that many
        // and leaves the count in `size`.
        let r = unsafe {
            libc::sysctl(
                mib.as_mut_ptr(),
                3,
                buf.as_mut_ptr().cast(),
                &mut size,
                ptr::null_mut(),
                0,
            )
        };
        if r != 0 {
            return None;
        }
        // A full buffer may be cut, and a cut one holds the END of the area, not its start (measured 2026-11-06: success,
        // argc right, exec path gone). The area is kern.argmax (1 MiB) and a little at most: it fits by 4 MiB.
        if size < buf.len() {
            return (size > size_of::<i32>()).then(|| &buf[..size]);
        }
        if buf.len() >= 4 << 20 {
            return None;
        }
        let grown = buf.len() * 4;
        buf.resize(grown, 0);
    }
}

/// `sysctl KERN_PROCARGS2` data: argc (i32), the exec path, NUL padding, then the arguments, each ended by NUL (the
/// environment follows). Returns argc and the arguments, as many as argc and the data give (Swift `processArguments`).
fn arguments(data: &[u8]) -> Option<(usize, impl Iterator<Item = &[u8]>)> {
    let argc = usize::try_from(i32::from_ne_bytes(data.get(..4)?.try_into().ok()?)).ok()?;
    let data = &data[4..];
    let path_end = data.iter().position(|&b| b == 0).unwrap_or(data.len());
    let start = data[path_end..]
        .iter()
        .position(|&b| b != 0)
        .map_or(data.len(), |n| path_end + n);
    let mut rest = &data[start..];
    let args = std::iter::from_fn(move || {
        if rest.is_empty() {
            return None;
        }
        let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
        let arg = &rest[..end];
        rest = rest.get(end + 1..).unwrap_or_default();
        Some(arg)
    });
    Some((argc, args.take(argc)))
}

/// All arguments, `None` if fewer than argc. Invalid UTF-8 replaced (Swift `String(decoding:as:)`).
fn argument_strings(data: &[u8]) -> Option<Vec<String>> {
    let (argc, args) = arguments(data)?;
    let args: Vec<String> = args
        .map(|a| String::from_utf8_lossy(a).into_owned())
        .collect();
    (args.len() == argc).then_some(args)
}

/// argv[0], as `ps -o comm=` prints it; `None` without one (ps then shows `(p_comm)`).
fn program_name(data: &[u8]) -> Option<String> {
    let first = arguments(data)?.1.next()?;
    (!first.is_empty()).then(|| String::from_utf8_lossy(first).into_owned())
}

/// "ttys004" like `ps -o tty=`: `devname` of the controlling terminal, "??" without one. devname takes ~0.5 ms a call
/// (measured 2026-11-06, ps spends most of its 0.12 s there), so found names are kept: a device number keeps its name.
fn tty_name(dev: libc::dev_t) -> String {
    const NODEV: libc::dev_t = -1;
    static NAMES: Mutex<BTreeMap<libc::dev_t, String>> = Mutex::new(BTreeMap::new());
    if dev == NODEV {
        return "??".into();
    }
    let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(name) = names.get(&dev) {
        return name.clone();
    }
    let mut buf = [0 as c_char; 64];
    // SAFETY: devname_r writes at most `len` bytes, NUL included, into `buf` and returns `buf` or NULL.
    let found = unsafe { devname_r(dev, libc::S_IFCHR, buf.as_mut_ptr(), buf.len() as c_int) };
    if found.is_null() {
        return "??".into();
    }
    // SAFETY: not NULL, so it is `buf`, NUL-terminated by devname_r.
    let name = unsafe { CStr::from_ptr(found) }
        .to_string_lossy()
        .into_owned();
    names.insert(dev, name.clone());
    name
}

/// CPU time user + system in seconds, 0 when not readable (another user's process). `proc_taskinfo` counts Mach time
/// units: 125/3 ns each on Apple Silicon (mach_timebase_info, measured 2026-11-06), 1 ns on Intel.
fn cpu_seconds(pid: Pid) -> f64 {
    static NS_PER_UNIT: OnceLock<f64> = OnceLock::new();
    let ns_per_unit = *NS_PER_UNIT.get_or_init(|| {
        let mut tb = MachTimebase { numer: 0, denom: 0 };
        // SAFETY: writes the two fields of `tb`.
        let r = unsafe { mach_timebase_info(&mut tb) };
        if r == 0 && tb.denom != 0 {
            f64::from(tb.numer) / f64::from(tb.denom)
        } else {
            1.0
        }
    });
    // SAFETY: integers only; all zero is a valid value.
    let mut info: libc::proc_taskinfo = unsafe { std::mem::zeroed() };
    let size = size_of::<libc::proc_taskinfo>() as c_int;
    // SAFETY: `info` is writable for `size` bytes; proc_pidinfo writes at most that many.
    let got =
        unsafe { libc::proc_pidinfo(pid, libc::PROC_PIDTASKINFO, 0, (&raw mut info).cast(), size) };
    if got != size {
        return 0.0;
    }
    info.pti_total_user.saturating_add(info.pti_total_system) as f64 * ns_per_unit / 1e9
}

/// A C string in a fixed buffer: up to the first NUL or the buffer's end, invalid UTF-8 replaced (Swift
/// `String(cString:)`).
fn c_string(chars: &[c_char]) -> String {
    let bytes: Vec<u8> = chars
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

/// `struct kinfo_proc` (<sys/sysctl.h>), 648 bytes. libc has no binding for Apple, so only the fields read here are
/// named; the rest is padding at the header's offsets (measured with `offsetof`, macOS 27 SDK, 2026-11-06).
#[repr(C)]
#[derive(Clone, Copy)]
struct KinfoProc {
    /// `kp_proc.p_un.__p_starttime`.
    start_sec: libc::time_t,
    start_usec: i32,
    _a: [u8; 24],
    /// `kp_proc.p_stat`; `SZOMB` = 5.
    stat: u8,
    _a2: [u8; 3],
    /// `kp_proc.p_pid`.
    pid: Pid,
    _b: [u8; 199],
    /// `kp_proc.p_comm`, MAXCOMLEN + 1.
    comm: [c_char; 17],
    _c: [u8; 300],
    /// `kp_eproc.e_ppid`.
    ppid: Pid,
    _d: [u8; 8],
    /// `kp_eproc.e_tdev`.
    tdev: libc::dev_t,
    _e: [u8; 72],
}

const _: () = assert!(
    size_of::<KinfoProc>() == 648
        && offset_of!(KinfoProc, start_usec) == 8
        && offset_of!(KinfoProc, stat) == 36
        && offset_of!(KinfoProc, pid) == 40
        && offset_of!(KinfoProc, comm) == 243
        && offset_of!(KinfoProc, ppid) == 560
        && offset_of!(KinfoProc, tdev) == 572
);

impl KinfoProc {
    // SAFETY: integers and byte arrays only; all zero is a valid value.
    const ZERO: KinfoProc = unsafe { std::mem::zeroed() };
}

/// `mach_timebase_info_data_t` (libc's binding is deprecated).
#[repr(C)]
struct MachTimebase {
    numer: u32,
    denom: u32,
}

unsafe extern "C" {
    /// <mach/mach_time.h>.
    fn mach_timebase_info(info: *mut MachTimebase) -> c_int;
    /// <stdlib.h>: the thread-safe devname (libc binds only `devname`, with its static buffer).
    fn devname_r(dev: libc::dev_t, mode: libc::mode_t, buf: *mut c_char, len: c_int)
    -> *mut c_char;
}

#[cfg(test)]
mod tests {
    use std::os::unix::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Instant;

    use lookout_core::mapping::engine_of;
    use lookout_core::model::Engine;

    use super::*;

    /// First line of every test: nothing here may write to the real ~/Library/Logs/Lookout.log (nettop and lsof log
    /// when they fail). `Once`: the variable is set before any of these tests goes on, setting it while another thread
    /// reads the environment is undefined.
    fn setup() {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            if std::env::var_os("LOOKOUT_LOG").is_none() {
                let log = std::env::temp_dir()
                    .join(format!("lookout-sys-test-{}.log", std::process::id()));
                // SAFETY: inside call_once, which every test of this module passes before it does anything else.
                unsafe { std::env::set_var("LOOKOUT_LOG", log) };
            }
        });
        let real = format!("{}/Library/Logs/Lookout.log", crate::home());
        assert_ne!(crate::log_path(), real, "tests never log into the real log");
    }

    fn me() -> Pid {
        std::process::id() as Pid
    }

    /// A ps line: `N` columns without spaces, then the rest (comm may contain spaces).
    fn columns<const N: usize>(line: &str) -> Option<([&str; N], &str)> {
        let mut rest = line;
        let mut out = [""; N];
        for f in &mut out {
            rest = rest.trim_start();
            let end = rest.find(' ')?;
            *f = &rest[..end];
            rest = &rest[end..];
        }
        Some((out, rest.trim()))
    }

    /// The KERN_PROCARGS2 layout: argc, exec path, padding, arguments, environment.
    #[test]
    fn procargs_layout() {
        let data = |argc: i32, rest: &[u8]| [&argc.to_ne_bytes()[..], rest].concat();
        let two = data(
            2,
            b"/opt/homebrew/bin/grok\0\0\0\0grok\0--cwd\0HOME=/Users/k\0\0",
        );
        assert_eq!(
            argument_strings(&two),
            Some(vec!["grok".to_string(), "--cwd".to_string()])
        );
        assert_eq!(program_name(&two).as_deref(), Some("grok"));
        // An empty argument counts; data that ends before argc arguments is no answer.
        assert_eq!(
            argument_strings(&data(2, b"/bin/x\0\0a\0\0")),
            Some(vec!["a".to_string(), String::new()])
        );
        assert_eq!(argument_strings(&data(3, b"/bin/x\0\0a\0b\0")), None);
        assert_eq!(argument_strings(&data(-1, b"/bin/x\0\0a\0")), None);
        // No argv[0] (argc 0, nothing after the path): the caller takes p_comm, as ps does.
        assert_eq!(program_name(&data(0, b"/bin/x\0\0a\0")), None);
        assert_eq!(program_name(&data(1, b"/bin/x\0\0")), None);
    }

    /// The rows say what `ps -Ao pid=,ppid=,tty=,comm=` says: ppid and tty for every process, comm for the user's own
    /// (only root reads the arguments of others, and ps is setuid root). Between the two snapshots a process may exec
    /// or lose its parent: at most 2 differences.
    #[test]
    fn rows_match_ps() {
        setup();
        let ps = Command::new("/bin/ps")
            .args(["-Ao", "pid=,uid=,ppid=,tty=,comm="])
            .output()
            .unwrap();
        let start = Instant::now();
        let rows = ps_rows().expect("process table readable");
        eprintln!("ps_rows: {} rows in {:?}", rows.len(), start.elapsed());
        let ours: BTreeMap<Pid, &PsRow> = rows.iter().map(|r| (r.pid, r)).collect();
        // SAFETY: no preconditions.
        let uid = unsafe { libc::geteuid() }.to_string();
        let (mut own, mut same, mut differ) = (0, 0, vec![]);
        for line in String::from_utf8_lossy(&ps.stdout).lines() {
            let Some(([pid, owner, ppid, tty], comm)) = columns::<4>(line) else {
                continue;
            };
            let Some(row) = pid.parse::<Pid>().ok().and_then(|p| ours.get(&p)) else {
                continue;
            };
            let mine = owner == uid;
            own += usize::from(mine);
            // ps writes "(p_comm)" where it cannot read the arguments either (a zombie).
            let comm_same = !mine || comm == row.comm || comm == format!("({})", row.comm);
            if ppid == row.ppid.to_string() && tty == row.tty && comm_same {
                same += 1;
            } else {
                differ.push((line.to_string(), (*row).clone()));
            }
        }
        eprintln!(
            "{same} rows like ps ({own} own with comm), changed between the snapshots: {differ:#?}"
        );
        assert!(own > 10 && same > 100, "{own} own, {same} compared");
        assert!(differ.len() <= 2, "{} rows differ", differ.len());
    }

    /// `comm` is the program as started, like ps: a symlink called `claude` stays `claude` (proc_pidpath answers
    /// /bin/sleep; Claude runs as ~/.local/bin/claude -> .../versions/2.1.319), and argv[0] counts, not the exec path.
    #[test]
    fn comm_is_the_name_as_started() {
        setup();
        let dir = std::env::temp_dir().join(format!("lookout-sys-test-{}-comm", me()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("claude");
        std::os::unix::fs::symlink("/bin/sleep", &link).unwrap();
        let mut via_link = Command::new(&link).arg("10").spawn().unwrap();
        let mut renamed = Command::new("/bin/sleep")
            .arg0("claude")
            .arg("10")
            .spawn()
            .unwrap();
        // spawn returns after the exec.
        let rows = ps_rows().unwrap();
        let comm_of = |child: &std::process::Child| {
            rows.iter()
                .find(|r| r.pid == child.id() as Pid)
                .map(|r| r.comm.clone())
        };
        let (link_comm, renamed_comm) = (comm_of(&via_link), comm_of(&renamed));
        let ps_renamed = Command::new("/bin/ps")
            .args(["-o", "comm=", "-p", &renamed.id().to_string()])
            .output()
            .unwrap();
        for child in [&mut via_link, &mut renamed] {
            let _ = child.kill();
            let _ = child.wait();
        }
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(link_comm.as_deref(), link.to_str());
        let row = PsRow {
            pid: 1,
            ppid: 0,
            tty: "ttys001".into(),
            cpu: 0.0,
            comm: link_comm.unwrap(),
        };
        assert_eq!(engine_of(&row), Some(Engine::Claude));
        assert_eq!(renamed_comm.as_deref(), Some("claude"));
        assert_eq!(
            String::from_utf8_lossy(&ps_renamed.stdout).trim(),
            "claude",
            "ps shows argv[0] too"
        );
    }

    /// CPU time in seconds, Mach time units converted: the own process against getrusage after burning 0.3 s.
    #[test]
    fn cpu_time_like_getrusage() {
        setup();
        let rusage = || {
            // SAFETY: all zero is a valid rusage; getrusage fills it.
            let ru = unsafe {
                let mut ru: libc::rusage = std::mem::zeroed();
                libc::getrusage(libc::RUSAGE_SELF, &mut ru);
                ru
            };
            let secs = |t: libc::timeval| t.tv_sec as f64 + f64::from(t.tv_usec) / 1e6;
            secs(ru.ru_utime) + secs(ru.ru_stime)
        };
        let begin = rusage();
        while rusage() - begin < 0.3 {}
        let before = rusage();
        let ours = ps_rows()
            .unwrap()
            .into_iter()
            .find(|r| r.pid == me())
            .unwrap()
            .cpu;
        let after = rusage();
        eprintln!("cpu: ps_rows {ours} s, getrusage {before} to {after} s");
        assert!(
            ours >= 0.3 && ours > before - 0.5 && ours < after + 0.5,
            "ps_rows {ours} s, getrusage {before} to {after} s"
        );
    }

    /// Swift testProcessStartDate: launchd's start as `ps -o lstart` shows it, the own start in the past, none for an
    /// unknown pid; launchd (kill: EPERM) and this process live, an unknown pid and process group 0 do not.
    #[test]
    fn process_start_like_ps() {
        setup();
        let ps = Command::new("/bin/ps")
            .args(["-o", "lstart=", "-p", "1"])
            .env("LC_ALL", "C")
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&ps.stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        let from_ps = local_time(&text).unwrap_or_else(|| panic!("ps -o lstart: {text:?}"));
        let launchd = process_start(1).expect("launchd (root) has a start");
        assert!(
            (launchd - from_ps).abs() < 1.0,
            "{launchd} vs ps {from_ps} ({text})"
        );
        assert!(process_start(me()).unwrap() <= crate::now());
        assert_eq!(process_start(99_999_990), None);
        assert!(process_alive(1) && process_alive(me()));
        assert!(!process_alive(99_999_990) && !process_alive(0));
    }

    /// "Mon Oct 26 10:08:58 2026", local time -> seconds since the epoch.
    fn local_time(text: &str) -> Option<f64> {
        let text = std::ffi::CString::new(text).ok()?;
        // SAFETY: all zero is a valid tm; strptime reads two NUL-terminated strings and fills `tm`, mktime normalizes it.
        unsafe {
            let mut tm: libc::tm = std::mem::zeroed();
            if libc::strptime(text.as_ptr(), c"%a %b %d %H:%M:%S %Y".as_ptr(), &mut tm).is_null() {
                return None;
            }
            tm.tm_isdst = -1;
            Some(libc::mktime(&mut tm) as f64)
        }
    }

    /// Swift testLaunchPromptIsNotKevins (last assert): the own start arguments; and the own working folder.
    #[test]
    fn own_arguments_and_folder() {
        setup();
        assert_eq!(
            process_arguments(me()),
            Some(std::env::args().collect::<Vec<_>>())
        );
        assert_eq!(
            process_cwd(me()).map(PathBuf::from),
            std::env::current_dir().ok()
        );
        assert_eq!(process_arguments(99_999_990), None);
        assert_eq!(process_cwd(99_999_990), None);
    }

    /// The title reaches a terminal (a pty of its own here) as exactly ESC ] 0 ; title BEL, a missing one refuses; lsof
    /// sees a listening socket of this process; nettop runs; the full name is the one `id -F` shows.
    #[test]
    fn small_services() {
        setup();
        assert!(!write_osc_title("ttys999", "x"), "no such terminal");
        let (mut master, mut slave) = (0, 0);
        let mut name = [0 as c_char; 128];
        // SAFETY: valid out-pointers; `name` holds more than a /dev/ttysNNN path; no termios, no window size.
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                name.as_mut_ptr(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        assert_eq!(opened, 0, "openpty");
        let tty = c_string(&name);
        let wrote = write_osc_title(tty.trim_start_matches("/dev/"), "Hallo");
        let mut buf = [0u8; 64];
        // SAFETY: descriptors from openpty, each closed once; `buf` is writable for its length. Non-blocking: a missing
        // title fails the test instead of hanging it.
        let got = unsafe {
            libc::fcntl(master, libc::F_SETFL, libc::O_NONBLOCK);
            let got = libc::read(master, buf.as_mut_ptr().cast(), buf.len());
            libc::close(slave);
            libc::close(master);
            got
        };
        assert!(wrote, "written to {tty}");
        assert_eq!(&buf[..got.max(0) as usize], b"\x1b]0;Hallo\x07");

        assert_eq!(listening_pids(&[]), Some(BTreeSet::new()));
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let listening = listening_pids(&[me()]);
        drop(listener);
        assert!(listening.is_some_and(|p| p.contains(&me())));

        let start = Instant::now();
        let net = nettop_usage();
        eprintln!(
            "nettop: {:?} processes in {:?}",
            net.as_ref().map(BTreeMap::len),
            start.elapsed()
        );
        assert!(net.is_some(), "nettop runs");

        // Locked or not depends on the moment; it answers.
        let _ = screen_locked();
        let id = Command::new("/usr/bin/id").arg("-F").output().unwrap();
        assert_eq!(full_user_name(), String::from_utf8_lossy(&id.stdout).trim());

        let (procs, rows) = engine_processes(&BTreeSet::new()).unwrap();
        assert!(rows.iter().any(|r| r.pid == me()));
        assert!(
            procs
                .iter()
                .all(|p| p.tty.starts_with("ttys") && !p.in_terminal)
        );
    }
}

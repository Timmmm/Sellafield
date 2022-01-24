use anyhow::{anyhow, bail, Context, Result};
use argh::FromArgs;
use errno::{errno, set_errno, Errno};
use fs_err as fs;
use rhai::{Engine, OptimizationLevel, Scope};
use std::{
    ffi::{CStr, OsStr},
    io::{self, BufWriter, Read, Write},
    os::unix::ffi::OsStrExt,
    path::{Path, PathBuf},
    rc::Rc,
    cell::RefCell,
};

// # `core_pattern` options
//
// %%  A single % character.
// %c  Core file size soft resource limit of crashing process
//     (since Linux 2.6.24).
// %d  Dump mode—same as value returned by prctl(2)
//     PR_GET_DUMPABLE (since Linux 3.7).
// %e  The process or thread's comm value, which typically is
//     the same as the executable filename (without path prefix,
//     and truncated to a maximum of 15 characters), but may
//     have been modified to be something different; see the
//     discussion of /proc/[pid]/comm and
//     /proc/[pid]/task/[tid]/comm in proc(5).
// %E  Pathname of executable, with slashes ('/') replaced by
//     exclamation marks ('!') (since Linux 3.0).
// %g  Numeric real GID of dumped process.
// %h  Hostname (same as nodename returned by uname(2)).
// %i  TID of thread that triggered core dump, as seen in the
//     PID namespace in which the thread resides (since Linux
//     3.18).
// %I  TID of thread that triggered core dump, as seen in the
//     initial PID namespace (since Linux 3.18).
// %p  PID of dumped process, as seen in the PID namespace in
//     which the process resides.
// %P  PID of dumped process, as seen in the initial PID
//     namespace (since Linux 3.12).
// %s  Number of signal causing dump.
// %t  Time of dump, expressed as seconds since the Epoch,
//     1970-01-01 00:00:00 +0000 (UTC).
// %u  Numeric real UID of dumped process.

// Note there's also %f for the filename, but it's only from 2019 so we won't use it.

#[derive(FromArgs)]
/// Configurably write core dumps. This can be used to avoid filling up HOME and
/// to make core dumps world-readable.
/// See https://man7.org/linux/man-pages/man5/core.5.html
struct Opts {
    /// numeric user ID of the crashed process. Use '%u' for this.
    #[argh(option, short = 'u')]
    uid: u32,

    /// process ID of the crashed process. Use '%p' or '%P' for this.
    #[argh(option, short = 'p')]
    pid: u32,

    /// unix time in millis when the process crashed. Use '%t' for this.
    #[argh(option, short = 't')]
    time: u32,

    /// path to the executable. !'s are replaced with /. Use %E for this.
    #[argh(option, short = 'E')]
    exe: String,

    /// core file size limit. Use %c for this.
    #[argh(option, short = 'c')]
    core_limit: u64,

    /// location of the config file that determines output location and permissions
    #[argh(option)]
    config: PathBuf,
}

#[derive(Debug, Clone, Default)]
struct Config {
    output_path: String,
    permissions: u64,
}

type SharedConfig = Rc<RefCell<Config>>;

fn timestamp() -> u128 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).expect("It's before 1970!").as_millis()
}

fn main() -> Result<()> {
    if let Err(e) = run() {
        // Stdout and Stderr go nowhere when run as a core_pattern so write this to a file.
        let mut file = fs::File::create(format!("/tmp/sellafield_{}.log", timestamp()))?;
        writeln!(file, "{}", e)?;
        return Err(e);
    }
    Ok(())
}

fn run() -> Result<()> {
    let opts: Opts = try_from_env().map_err(|e| anyhow!("{}", e.output))?;

    // The kernel determines the minimum size for the specific output format
    // (e.g. 4kB for ELF), but we'll just do something simpler.
    // 1 is some kind of special value so exclude that too.
    if opts.core_limit <= 1 {
        return Ok(());
    }

    // This runs as root by default, but we want to drop permissions to the
    // given UID.
    set_uid(opts.uid)?;

    // Wrangle the exe name which has / replaced with !. Better hope nobody puts
    // ! in their filenames!
    let full_exe = opts.exe.replace('!', "/");
    let exe = full_exe
        .rsplit_once('/')
        .and_then(|(_, exe)| Some(exe))
        .unwrap_or_default()
        .to_owned();

    // Get username & home directory.
    let user_details = get_user_details(opts.uid)?;

    // Run the config script to find the output path.
    let config = run_script(&opts, &full_exe, &exe, &user_details)?;

    if !config.output_path.is_empty() {
        // Copy stdin to the output path and set permissions.
        write_output(&config, opts.core_limit)?;
    }

    Ok(())
}

fn run_script(opts: &Opts, full_exe: &str, exe: &str, user_details: &UserDetails) -> Result<Config> {

    let mut engine = Engine::new();
    // We're only executing the script once so don't bother optimising it.
    engine.set_optimization_level(OptimizationLevel::None);

    // TODO: Sort out encodings. This is all wrong.
    let home = user_details.home.to_string_lossy().to_string();
    let username = user_details.username.clone();
    let uid = opts.uid;
    let pid = opts.pid;
    let time = opts.time;
    let full_exe = full_exe.to_owned();
    let exe = exe.to_owned();

    // Functions to get various details.
    engine.register_fn("home", move || home.clone());
    engine.register_fn("username", move || username.clone());
    engine.register_fn("uid", move || uid);
    engine.register_fn("pid", move || pid);
    engine.register_fn("time", move || time);
    engine.register_fn("full_exe", move || full_exe.clone());
    engine.register_fn("exe", move || exe.clone());

    // Config-setting functions.
    let config = SharedConfig::default();
    // Set sensible default.
    config.borrow_mut().permissions = 0o400u64;

    let cfg = config.clone();
    engine.register_fn("set_output_path", move |x| cfg.borrow_mut().output_path = x);
    let cfg = config.clone();
    engine.register_fn("set_permissions", move |x: i64| cfg.borrow_mut().permissions = x as u64);

    let mut scope = Scope::new();

    // Not sure why you can't use .context() here. It gives threading errors.
    engine
        .eval_file_with_scope::<()>(&mut scope, opts.config.clone())
        .map_err(|e| anyhow!("Sellafield config script execution error: {}", e))?;

    // Clone the config for simplicity.
    let config = config.borrow().clone();
    Ok(config)
}

fn write_output(config: &Config, core_limit: u64) -> Result<()> {
    // Set the umask otherwise it creates directories that are world-writable.
    set_umask(0o022);

    let output_path = Path::new(&config.output_path);

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let permissions_mode: u16 = config.permissions.try_into().map_err(|e| {
        anyhow!(
            "Sellafield config script returned invalid permissions mode: {}. {}",
            config.permissions,
            e
        )
    })?;

    // Set umask so we can set the requested bits of permissions_mode.
    set_umask(!(permissions_mode as libc::mode_t));

    // Write stdin to a file.
    // This has not been added to fs_err.
    // let out = File::options()
    let out = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&output_path)?;
    let mut out_writer = BufWriter::new(out);

    // Set file permissions. This may not be strictly necessary because of the
    // set_umask() call above but just to be sure...
    let permissions = make_permissions(permissions_mode as u32);
    fs::set_permissions(&output_path, permissions)
        .context("error setting core dump permissions")?;

    // Write stdin out to that file.
    let stdin = io::stdin();
    let mut stdin = stdin.lock();

    io::copy(&mut stdin.by_ref().take(core_limit), &mut out_writer)
        .context("error writing core dump")?;

    Ok(())
}

struct UserDetails {
    username: String,
    home: PathBuf,
}

#[cfg(unix)]
fn get_user_details(uid: u32) -> Result<UserDetails> {
    set_errno(Errno(0));
    let passwd = unsafe { libc::getpwuid(uid) };
    if passwd.is_null() {
        bail!("Error getting username for user ID {}: {}", uid, errno());
    }

    let pw_name_cstr: &CStr = unsafe { CStr::from_ptr((*passwd).pw_name) };
    let pw_name = latin1_to_string(pw_name_cstr.to_bytes());

    let pw_dir_cstr: &CStr = unsafe { CStr::from_ptr((*passwd).pw_dir) };
    let pw_dir = latin1_to_path(pw_dir_cstr.to_bytes());

    Ok(UserDetails {
        username: pw_name,
        home: pw_dir,
    })
}

#[cfg(unix)]
fn set_uid(uid: u32) -> Result<()> {
    // This is not strictly necessary in this case, but you can't be too
    // safe when dealing with C.
    set_errno(Errno(0));
    let rc = unsafe { libc::setuid(uid) };
    if rc != 0 {
        bail!("Error switching to user ID {}: {}", uid, errno());
    }
    Ok(())
}

#[cfg(unix)]
fn set_umask(mask: libc::mode_t) {
    // This always succeeds.
    unsafe {
        libc::umask(mask);
    }
}

fn latin1_to_string(s: &[u8]) -> String {
    s.iter().map(|&c| c as char).collect()
}

fn latin1_to_path(s: &[u8]) -> PathBuf {
    let os_str = OsStr::from_bytes(s);
    PathBuf::from(os_str)
}

#[cfg(unix)]
fn make_permissions(mode: u32) -> std::fs::Permissions {
    use std::os::unix::prelude::PermissionsExt;

    PermissionsExt::from_mode(mode)
}


/// Extract the base cmd from a path
fn cmd<'a>(default: &'a String, path: &'a String) -> &'a str {
    std::path::Path::new(path).file_name().map(|s| s.to_str()).flatten().unwrap_or(default.as_str())
}

/// Fallible version of argh::from_env().
pub fn try_from_env<T: argh::TopLevelCommand>() -> std::result::Result<T, argh::EarlyExit> {
    let strings: Vec<String> = std::env::args().collect();
    let cmd = cmd(&strings[0], &strings[0]);
    let strs: Vec<&str> = strings.iter().map(|s| s.as_str()).collect();
    T::from_args(&[cmd], &strs[1..])
}

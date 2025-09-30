/// Run in host environment, directly bind with IP address
use std::{
    cmp::min,
    fs::File,
    net,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use libc::{SIGKILL, SIGTERM};

use crate::{
    format::{FormatRunner, FormatRunnerFactory, Handle},
    get_program_name, Program, ProgramChild, ProgramStatus, Target,
};

fn get_binder_path() -> Option<PathBuf> {
    const CANONICALIZE_ERR_MSG: &str = "Failed to canonicalize libbinder.so path";
    let self_file = Path::new("/proc/self/exe").canonicalize();
    let libpath = match self_file {
        Ok(self_file) => {
            let libbinder = self_file.parent().unwrap().join("libbinder.so");
            if !libbinder.exists() {
                let libbinder = self_file
                    .parent()
                    .unwrap()
                    .join("deps")
                    .join("libbinder.so");
                if !libbinder.exists() {
                    None
                } else {
                    Some(libbinder.canonicalize().expect(CANONICALIZE_ERR_MSG))
                }
            } else {
                Some(libbinder.canonicalize().expect(CANONICALIZE_ERR_MSG))
            }
        }
        Err(_) => None,
    };
    let libpath = match libpath {
        Some(libpath) => libpath,
        None => {
            panic!(
                r#"libbinder.so not found. Please put it in same folder of bestbind.
You can download corresponding file from https://github.com/taoky/libbinder/releases"#
            );
        }
    };
    Some(libpath)
}

fn get_child(
    program: &Program,
    bind_ip: &str,
    upstream: &str,
    tmp_path: &Path,
    log_file: &File,
    binder: &Option<PathBuf>,
    extra: &Option<String>,
) -> ProgramChild {
    let tmp = tmp_path.as_os_str().to_string_lossy().to_string();
    let extra = shlex::split(extra.as_ref().unwrap_or(&"".to_string()))
        .expect("Failed to parse extra arguments");
    let mut cmd: Command;
    ProgramChild {
        child: match program {
            Program::Rsync => {
                cmd = std::process::Command::new("rsync");
                cmd.arg("-vP")
                    .arg("-rLptgoD")
                    .arg("--inplace")
                    .arg("--address")
                    .arg(bind_ip)
                    .arg(upstream)
                    .arg(tmp)
                    .args(extra)
            }
            Program::Curl => {
                cmd = std::process::Command::new("curl");
                cmd.arg("-o")
                    .arg(tmp)
                    .arg("--interface")
                    .arg(bind_ip)
                    .arg(upstream)
                    .args(extra)
            }
            Program::Wget => {
                cmd = std::process::Command::new("wget");
                cmd.arg("-O")
                    .arg(tmp)
                    .arg("--bind-address")
                    .arg(bind_ip)
                    .arg(upstream)
                    .args(extra)
            }
            Program::Git => {
                cmd = std::process::Command::new("git");
                cmd.env("LD_PRELOAD", binder.clone().unwrap())
                    .env("BIND_ADDRESS", bind_ip)
                    .arg("clone")
                    .arg("--bare")
                    .arg(upstream)
                    .arg(tmp)
            }
        }
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            log_file
                .try_clone()
                .expect("Clone log file descriptor failed (stdout)"),
        ))
        .stderr(Stdio::from(
            log_file
                .try_clone()
                .expect("Clone log file descriptor failed (stderr)"),
        ))
        .process_group(0) // Don't receive SIGINT from tty: we handle it ourselves (for rsync)
        .spawn()
        .unwrap_or_else(|_| {
            panic!(
                "Failed to spawn {} with timeout.",
                get_program_name(program)
            )
        }),
        program: *program,
    }
}

fn reap_all_children() {
    loop {
        unsafe {
            if libc::waitpid(-1, std::ptr::null_mut(), libc::WNOHANG) < 0 {
                break;
            }
        }
    }
}

fn kill_children(proc: &mut ProgramChild) -> ExitStatus {
    // Soundness requirement: the latest try_wait() should return Ok(None)
    // Elsewhere libc::kill may kill unrelated processes

    // rsync process model: we spawn "generator", and after receiving "file list"
    // generator spawns "receiver".
    // A race condition bug of rsync will cause receiver to hang for a long time
    // when both generator and receiver get SIGTERM/SIGINT/SIGHUP.
    // (See https://github.com/WayneD/rsync/issues/413 I posted)
    // So we seperate rsync from rsync-speedtest process group,
    // and just SIGTERM "generator" here, and let generator to SIGUSR1 receiver
    // and hoping that it will work
    // and well, I think that std::process::Child really should get a terminate() method!

    // git process model: git spawns some git-remote-https (for example) to do the networking work
    // and when getting SIGTERM, etc., git will do cleanup job and we cannot get actual data afterwards
    // So we have to kill the whole process group with the crudest way
    if proc.program != Program::Git {
        unsafe {
            libc::kill(proc.child.id() as i32, SIGTERM);
        }
    } else {
        unsafe {
            // SIGKILL the whole process group to cleanup git-remote-*
            libc::killpg(proc.child.id() as i32, SIGKILL);
        }
    }

    // let res = proc.child.wait().expect("program wait() failed");
    // Try waiting for 5 more seconds to let it cleanup
    let mut res: Option<ExitStatus> = None;
    for _ in 0..50 {
        if let Some(status) = proc
            .child
            .try_wait()
            .expect("try waiting for child process failed")
        {
            res = Some(status);
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    if res.is_none() {
        // Still not exited, kill it
        println!(
            "Killing {} with SIGKILL, as it is not exiting with SIGTERM.",
            get_program_name(&proc.program)
        );
        unsafe {
            libc::kill(proc.child.id() as i32, SIGKILL);
        }
        res = Some(proc.child.wait().expect("program wait() failed"));
    }
    // if receiver died before generator, the SIGCHLD handler of generator will help reap it
    // but we cannot rely on race condition to help do things right
    reap_all_children();

    res.unwrap()
}

pub struct IPFormatRunner {
    uses: Vec<Target>,
    binder_path: Option<PathBuf>,
    extra: Option<String>,
    program: Program,
    upstream: String,
}

pub struct IPFormatHandle {
    child: ProgramChild,
}

impl Handle for IPFormatHandle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> crate::ProgramStatus {
        // Reference adaptable timeout algorithm from
        // https://github.com/hniksic/rust-subprocess/blob/5e89ac093f378bcfc03c69bdb1b4bcacf4313ce4/src/popen.rs#L778
        // Licensed under MIT & Apache-2.0

        let start = Instant::now();
        let deadline = start + timeout;

        let mut delay = Duration::from_millis(1);
        let proc = &mut self.child;

        loop {
            let status = proc
                .child
                .try_wait()
                .expect("try waiting for child process failed");
            match status {
                Some(status) => {
                    return ProgramStatus {
                        status,
                        time: start.elapsed(),
                    }
                }
                None => {
                    if term.load(Ordering::SeqCst) {
                        let time = start.elapsed();
                        let status = kill_children(proc);
                        return ProgramStatus { status, time };
                    }

                    let now = Instant::now();
                    if now >= deadline {
                        let time = start.elapsed();
                        let status = kill_children(proc);
                        return ProgramStatus { status, time };
                    }

                    let remaining = deadline.duration_since(now);
                    std::thread::sleep(min(delay, remaining));
                    delay = min(delay * 2, Duration::from_millis(100));
                }
            }
        }
    }
}

impl FormatRunner for IPFormatRunner {
    type HandleType = dyn Handle;

    fn uses(&self) -> &Vec<crate::Target> {
        &self.uses
    }

    fn run(&self, target: &str, tmp_file: &mktemp::Temp, log: &File) -> Box<Self::HandleType> {
        Box::new(IPFormatHandle {
            child: get_child(
                &self.program,
                target,
                &self.upstream,
                tmp_file,
                log,
                &self.binder_path,
                &self.extra,
            ),
        })
    }
}

impl FormatRunnerFactory for IPFormatRunner {
    fn create(
        args: &crate::Args,
        profile: crate::Profile,
        program: crate::Program,
    ) -> Box<dyn FormatRunner<HandleType = dyn Handle>> {
        let mut uses: Vec<Target> = Vec::new();
        for (ip, comment) in profile.uses {
            let _ = ip.parse::<net::IpAddr>().expect("Invalid IP address");
            uses.push(Target { ip, comment });
        }

        let binder_path = if program == Program::Git {
            get_binder_path()
        } else {
            None
        };

        Box::new(IPFormatRunner {
            uses,
            binder_path,
            extra: args.extra.clone(),
            program,
            upstream: args.upstream.clone(),
        })
    }
}

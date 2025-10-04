/// Run in host environment, directly bind with IP address
use std::{
    fs::File,
    net,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use libc::{SIGKILL, SIGTERM};

use crate::{
    format::{get_program_args, wait_timeout, FormatRunner, FormatRunnerFactory, Handle},
    get_program_name, Program, ProgramChild, Target,
};

fn get_binder_path() -> PathBuf {
    let mut paths_to_check = vec!["/usr/lib/bestbind/libbinder.so".to_string()];
    if let Ok(env_path) = std::env::var("LIBBINDER_PATH") {
        paths_to_check.push(env_path);
    }

    let libpath = paths_to_check.iter().find_map(|p| {
        let path = Path::new(p);
        if path.exists() {
            Some(path.to_path_buf())
        } else {
            None
        }
    }).unwrap_or_else(|| {
        panic!(
            r"libbinder.so not found. Please put it in /usr/lib/bestbind/ or set LIBBINDER_PATH environment variable.
You can download corresponding file from https://github.com/taoky/libbinder/releases"
        )
    });
    libpath
}

fn get_child(
    program: Program,
    bind_ip: &str,
    upstream: &str,
    tmp_path: &Path,
    log_file: &File,
    binder: Option<&PathBuf>,
    extra: &[String],
) -> ProgramChild {
    let mut cmd: Command;
    let args = get_program_args(program, extra, upstream, tmp_path, Some(bind_ip));
    ProgramChild {
        child: match program {
            Program::Rsync => {
                cmd = std::process::Command::new("rsync");
                cmd.args(args)
            }
            Program::Curl => {
                cmd = std::process::Command::new("curl");
                cmd.args(args)
            }
            Program::Wget => {
                cmd = std::process::Command::new("wget");
                cmd.args(args)
            }
            Program::Git => {
                cmd = std::process::Command::new("git");
                cmd.env("LD_PRELOAD", binder.unwrap())
                    .env("BIND_ADDRESS", bind_ip)
                    .args(args)
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
        program,
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

pub struct IPFormatRunner {
    uses: Vec<Target>,
    binder_path: Option<PathBuf>,
    extra: Vec<String>,
    program: Program,
    upstream: String,
}

pub struct IPFormatHandle {
    child: ProgramChild,
}

impl Handle for IPFormatHandle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> crate::ProgramStatus {
        wait_timeout(self, timeout, &term)
    }

    fn child(&mut self) -> &mut ProgramChild {
        &mut self.child
    }

    fn kill_children(&mut self) -> ExitStatus {
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

        let proc = self.child();
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
                get_program_name(proc.program)
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
}

impl FormatRunner for IPFormatRunner {
    type HandleType = dyn Handle;

    fn uses(&self) -> &Vec<crate::Target> {
        &self.uses
    }

    fn run(&self, target: &str, tmp_path: &mktemp::Temp, log: &File) -> Box<Self::HandleType> {
        Box::new(IPFormatHandle {
            child: get_child(
                self.program,
                target,
                &self.upstream,
                tmp_path,
                log,
                self.binder_path.as_ref(),
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
            uses.push(Target {
                network: ip,
                comment,
            });
        }

        let binder_path = if program == Program::Git {
            Some(get_binder_path())
        } else {
            None
        };

        Box::new(Self {
            uses,
            binder_path,
            extra: args.extra.clone(),
            program,
            upstream: args.upstream.clone(),
        })
    }
}

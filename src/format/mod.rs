use std::{
    cmp::min,
    fs::File,
    path::Path,
    process::ExitStatus,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

use mktemp::Temp;

use crate::{Args, Format, Profile, Program, ProgramChild, ProgramStatus};

mod docker;
mod ip;

pub trait Handle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> ProgramStatus;
}

pub trait FormatRunner {
    type HandleType: Handle + ?Sized + 'static;

    fn uses(&self) -> &Vec<crate::Target>;
    fn run(&self, target: &str, tmp_path: &Temp, log: &File) -> Box<Self::HandleType>;
}

trait FormatRunnerFactory {
    fn create(
        args: &Args,
        profile: Profile,
        program: Program,
    ) -> Box<dyn FormatRunner<HandleType = dyn Handle>>;
}

pub fn get_runner(
    format: Format,
    args: &Args,
    profile: Profile,
    program: Program,
) -> Box<dyn FormatRunner<HandleType = dyn Handle>> {
    match format {
        Format::IP => ip::IPFormatRunner::create(args, profile, program),
        Format::Docker => docker::DockerFormatRunner::create(args, profile, program),
    }
}

fn get_program_args(
    program: Program,
    extra: &[String],
    upstream: &str,
    tmp: &Path,
    bind_ip: Option<&str>,
) -> Vec<String> {
    let tmp = tmp.as_os_str().to_string_lossy().to_string();
    let mut result = Vec::new();
    match program {
        Program::Rsync => {
            result.push("-vP".to_string());
            result.push("-rLptgoD".to_string());
            result.push("--inplace".to_string());
            if let Some(ip) = bind_ip {
                result.push("--address".to_string());
                result.push(ip.to_string());
            }
            result.push(upstream.to_string());
            result.push(tmp);
            result.extend(extra.iter().cloned());
        }
        Program::Curl => {
            result.push("-o".to_string());
            result.push(tmp);
            if let Some(ip) = bind_ip {
                result.push("--interface".to_string());
                result.push(ip.to_string());
            }
            result.push(upstream.to_string());
            result.extend(extra.iter().cloned());
        }
        Program::Wget => {
            result.push("-O".to_string());
            result.push(tmp);
            if let Some(ip) = bind_ip {
                result.push("--bind-address".to_string());
                result.push(ip.to_string());
            }
            result.push(upstream.to_string());
            result.extend(extra.iter().cloned());
        }
        Program::Git => {
            // Note that git does not support binding IP natively
            result.push("clone".to_string());
            result.push("--bare".to_string());
            result.push(upstream.to_string());
            result.push(tmp);
        }
    }

    result
}

fn wait_timeout(
    proc: &mut ProgramChild,
    timeout: Duration,
    term: &Arc<AtomicBool>,
    kill: fn(&mut ProgramChild) -> ExitStatus,
) -> crate::ProgramStatus {
    // Reference adaptable timeout algorithm from
    // https://github.com/hniksic/rust-subprocess/blob/5e89ac093f378bcfc03c69bdb1b4bcacf4313ce4/src/popen.rs#L778
    // Licensed under MIT & Apache-2.0

    let start = Instant::now();
    let deadline = start + timeout;

    let mut delay = Duration::from_millis(1);

    loop {
        let status = proc
            .child
            .try_wait()
            .expect("try waiting for child process failed");
        if let Some(status) = status {
            return ProgramStatus {
                status,
                time: start.elapsed(),
            };
        }

        if term.load(Ordering::SeqCst) {
            let time = start.elapsed();
            let status = kill(proc);
            return ProgramStatus { status, time };
        }

        let now = Instant::now();
        if now >= deadline {
            let time = start.elapsed();
            let status = kill(proc);
            return ProgramStatus { status, time };
        }

        let remaining = deadline.duration_since(now);
        std::thread::sleep(min(delay, remaining));
        delay = min(delay * 2, Duration::from_millis(100));
    }
}

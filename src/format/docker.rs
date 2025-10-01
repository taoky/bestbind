use std::{
    fs::File,
    os::unix::process::ExitStatusExt,
    process::ExitStatus,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

/// Run with docker, by specifying docker network
use crate::{
    format::{get_program_args, wait_timeout, FormatRunner, FormatRunnerFactory, Handle},
    Program, ProgramChild, Target,
};

pub struct DockerFormatHandle {
    child: ProgramChild,
}
pub struct DockerFormatRunner {
    docker: String,
    uses: Vec<crate::Target>,
    extra: Vec<String>,
    program: Program,
    upstream: String,
}

fn kill_container(child: &mut ProgramChild) -> ExitStatus {
    let _ = std::process::Command::new("docker")
        .args(["kill", &child.child.id().to_string()])
        .status();
    let _ = child.child.kill();
    let _ = child.child.wait();

    ExitStatus::from_raw(128 + libc::SIGKILL)
}

impl Handle for DockerFormatHandle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> crate::ProgramStatus {
        wait_timeout(&mut self.child, timeout, &term, kill_container)
    }
}

impl FormatRunner for DockerFormatRunner {
    type HandleType = dyn Handle;

    fn uses(&self) -> &Vec<crate::Target> {
        &self.uses
    }

    fn run(&self, target: &str, tmp_path: &mktemp::Temp, log: &File) -> Box<Self::HandleType> {
        let args = get_program_args(self.program, &self.extra, &self.upstream, tmp_path, None);
        let cmd = std::process::Command::new(&self.docker)
            .arg("run")
            .arg("--rm")
            .arg("--network")
            .arg(target)
            .arg("-v")
            .arg(format!("{}:/data", tmp_path.as_os_str().to_string_lossy()))
            .arg(self.program.to_string())
            .args(args)
            .stdout(log.try_clone().expect("Failed to clone log file"))
            .stderr(log.try_clone().expect("Failed to clone log file"))
            .stdin(std::process::Stdio::null())
            .spawn()
            .expect("Failed to start docker process");
        Box::new(DockerFormatHandle {
            child: ProgramChild {
                child: cmd,
                program: self.program,
            },
        })
    }
}

impl FormatRunnerFactory for DockerFormatRunner {
    fn create(
        args: &crate::Args,
        profile: crate::Profile,
        program: crate::Program,
    ) -> Box<dyn FormatRunner<HandleType = dyn Handle>> {
        let mut uses: Vec<Target> = Vec::new();
        for (network, comment) in profile.uses {
            uses.push(Target { network, comment });
        }
        let docker = profile.docker;
        // Check if an image exists
        if let Err(e) = std::process::Command::new(&docker)
            .args(["image", "inspect", &profile.image])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
        {
            println!("Failed to inspect docker image {}: {}", &profile.image, e);
            println!("Try pulling the image...");
            std::process::Command::new(&docker)
                .args(["pull", &profile.image])
                .status()
                .expect("Failed to pull docker image");
        }
        Box::new(Self {
            docker,
            uses,
            extra: args.extra.clone(),
            program,
            upstream: args.upstream.clone(),
        })
    }
}

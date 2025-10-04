use std::{
    fs::File,
    os::unix::process::ExitStatusExt,
    process::ExitStatus,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use rand::{distr::Alphanumeric, Rng};

/// Run with docker, by specifying docker network
use crate::{
    format::{get_program_args, wait_timeout, FormatRunner, FormatRunnerFactory, Handle},
    Program, ProgramChild, Target,
};

pub struct DockerFormatHandle {
    child: ProgramChild,
    ctr_name: String,
    docker: String,
}
pub struct DockerFormatRunner {
    docker: String,
    image: String,
    uses: Vec<crate::Target>,
    extra: Vec<String>,
    program: Program,
    upstream: String,
}

impl Handle for DockerFormatHandle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> crate::ProgramStatus {
        wait_timeout(self, timeout, &term)
    }

    fn child(&mut self) -> &mut ProgramChild {
        &mut self.child
    }

    fn kill_children(&mut self) -> ExitStatus {
        self.child
            .child
            .kill()
            .expect("Failed to kill child process");
        self.child
            .child
            .wait()
            .expect("Failed to wait child process");
        let status = std::process::Command::new(&self.docker)
            .args(["kill", self.ctr_name.as_str()])
            .status()
            .expect("Failed to kill docker container");
        assert!(
            status.success(),
            "Failed to kill docker container {}, exit code: {}",
            self.ctr_name,
            status.code().unwrap_or(-1)
        );

        ExitStatus::from_raw(128 + libc::SIGKILL)
    }
}

impl FormatRunner for DockerFormatRunner {
    type HandleType = dyn Handle;

    fn uses(&self) -> &Vec<crate::Target> {
        &self.uses
    }

    fn run(&self, target: &str, tmp_path: &mktemp::Temp, log: &File) -> Box<Self::HandleType> {
        let args = get_program_args(self.program, &self.extra, &self.upstream, tmp_path, None);
        let ctr_name = format!(
            "bestbind-{}",
            rand::rng()
                .sample_iter(&Alphanumeric)
                .take(16)
                .map(char::from)
                .collect::<String>()
        );
        let tmp = tmp_path.as_os_str().to_string_lossy();
        let cmd = std::process::Command::new(&self.docker)
            .arg("run")
            .arg("--name")
            .arg(&ctr_name)
            .arg("--rm")
            .arg("--network")
            .arg(target)
            .arg("-v")
            .arg(format!("{tmp}:{tmp}"))
            .arg(&self.image)
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
            ctr_name,
            docker: self.docker.clone(),
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
        let status = std::process::Command::new(&docker)
            .args(["image", "inspect", &profile.image])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .expect("failed to inspect docker image");
        if !status.success() {
            println!("Failed to inspect docker image {}", &profile.image);
            println!("Try pulling the image...");
            let status = std::process::Command::new(&docker)
                .args(["pull", &profile.image])
                .status()
                .expect("Failed to pull docker image");
            assert!(
                status.success(),
                "Failed to pull docker image {}, exit code: {}",
                &profile.image,
                status.code().unwrap_or(-1)
            );
        }
        Box::new(Self {
            docker,
            image: profile.image,
            uses,
            extra: args.extra.clone(),
            program,
            upstream: args.upstream.clone(),
        })
    }
}

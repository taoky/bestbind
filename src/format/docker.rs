use std::{
    fs::File,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

/// Run with docker, by specifying docker network
use crate::{
    format::{FormatRunner, FormatRunnerFactory, Handle},
    Target,
};

pub struct DockerFormatHandle;
pub struct DockerFormatRunner {
    docker: String,
    uses: Vec<crate::Target>,
    extra: Vec<String>,
    program: crate::Program,
    upstream: String,
}

impl Handle for DockerFormatHandle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> crate::ProgramStatus {
        unimplemented!()
    }
}

impl FormatRunner for DockerFormatRunner {
    type HandleType = dyn Handle;

    fn uses(&self) -> &Vec<crate::Target> {
        &self.uses
    }

    fn run(&self, target: &str, tmp_file: &mktemp::Temp, log: &File) -> Box<Self::HandleType> {
        unimplemented!()
    }
}

impl FormatRunnerFactory for DockerFormatRunner {
    fn create(
        args: &crate::Args,
        profile: crate::Profile,
        program: crate::Program,
    ) -> Box<dyn FormatRunner<HandleType = dyn Handle>> {
        let mut uses: Vec<Target> = Vec::new();
        for (network, comment) in profile.uses.into_iter() {
            uses.push(Target { network, comment });
        }
        let docker = profile.docker;
        // Check if an image exists
        if let Err(e) = std::process::Command::new(&docker)
            .args(&["image", "inspect", &profile.image])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).status()
        {
            println!("Failed to inspect docker image {}: {}", &profile.image, e);
            println!("Try pulling the image...");
            std::process::Command::new(&docker)
                .args(&["pull", &profile.image])
                .status()
                .expect("Failed to pull docker image");
        }
        Box::new(DockerFormatRunner {
            docker,
            uses,
            extra: args.extra.clone(),
            program,
            upstream: args.upstream.clone(),
        })
    }
}

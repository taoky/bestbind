use std::{
    fs::File,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

/// Run with docker, by specifying docker network
use crate::{format::{FormatRunner, FormatRunnerFactory, Handle}, Target};

pub struct DockerFormatHandle;
pub struct DockerFormatRunner {
    uses: Vec<crate::Target>,
    extra: Option<String>,
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
        for (ip, comment) in profile.uses.into_iter() {
            uses.push(Target { ip, comment });
        }
        Box::new(DockerFormatRunner {
            uses,
            extra: args.extra.clone(),
            program,
            upstream: args.upstream.clone(),
        })
    }
}

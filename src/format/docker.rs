use std::{
    fs::File,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

/// Run with docker, by specifying docker network
use crate::format::{FormatRunner, FormatRunnerFactory, Handle};

pub struct DockerFormatHandle;
pub struct DockerFormatRunner;

impl Handle for DockerFormatHandle {
    fn wait_timeout(&mut self, timeout: Duration, term: Arc<AtomicBool>) -> crate::ProgramStatus {
        unimplemented!()
    }
}

impl FormatRunner for DockerFormatRunner {
    type HandleType = DockerFormatHandle;

    fn uses(&self) -> &Vec<crate::Target> {
        unimplemented!()
    }

    fn run(&self, target: &str, tmp_file: &mktemp::Temp, log: &File) -> Box<DockerFormatHandle> {
        unimplemented!()
    }
}

impl FormatRunnerFactory for DockerFormatRunner {
    fn create(
        args: &crate::Args,
        profile: crate::Profile,
        program: crate::Program,
    ) -> Box<dyn FormatRunner<HandleType = dyn Handle>> {
        unimplemented!()
    }
}

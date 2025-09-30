use std::{
    fs::File,
    sync::{atomic::AtomicBool, Arc},
    time::Duration,
};

use mktemp::Temp;

use crate::{Args, Format, Profile, Program, ProgramStatus};

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

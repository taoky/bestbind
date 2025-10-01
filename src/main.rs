#![warn(clippy::all, clippy::pedantic, clippy::nursery)]
#![allow(
    clippy::cast_precision_loss,
    clippy::if_not_else,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::cast_possible_wrap,
    clippy::too_many_lines,
    clippy::option_if_let_else
)]

use std::{
    collections::HashMap,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{self, ExitStatus},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use anyhow::Result;
use clap::{Parser, ValueEnum};
use serde::Deserialize;
use signal_hook::consts::{SIGINT, SIGTERM};
use xdg::BaseDirectories;

use crate::format::get_runner;

mod format;

#[derive(Debug, ValueEnum, Clone, Copy, PartialEq)]
enum Program {
    Rsync,
    Wget,
    Curl,
    Git,
}

impl std::fmt::Display for Program {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Rsync => "rsync",
            Self::Wget => "wget",
            Self::Curl => "curl",
            Self::Git => "git",
        };
        write!(f, "{s}")
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Format {
    IP,
    Docker,
}

impl<'de> Deserialize<'de> for Format {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s: String = Deserialize::deserialize(deserializer)?;
        let s = s.to_lowercase();
        match s.as_str() {
            "ip" => Ok(Self::IP),
            "docker" => Ok(Self::Docker),
            _ => Err(serde::de::Error::custom(format!(
                "Unknown format: {s}. Supported formats: ip, docker"
            ))),
        }
    }
}

fn default_docker() -> String {
    "docker".to_string()
}

#[derive(Debug, Deserialize, Clone)]
struct Profile {
    format: Format,
    #[serde(default)]
    image: String, // Docker image name, only used in Docker format
    #[serde(default = "default_docker")]
    docker: String, // The "Docker" command, default to "docker".
    // A possible alternative is "podman"
    uses: HashMap<String, String>, // IP or Docker network => comment
}

#[derive(Parser, Debug)]
#[clap(about, version)]
struct Args {
    /// Profile name in config file. If not given, it will use "default" profile
    #[clap(long, default_value = "default")]
    profile: String,

    /// Config file (IP list) path. Select order is bestbind.conf in XDG config,
    /// then ~/.bestbind.conf, then /etc/bestbind.conf
    #[clap(short, long)]
    config: Option<String>,

    /// Passes number
    #[clap(short, long, default_value = "3")]
    pass: usize,

    /// Timeout (seconds)
    #[clap(short, long, default_value = "30")]
    timeout: usize,

    /// Tmp file path. Default to `env::temp_dir()` (/tmp in Linux system)
    #[clap(long)]
    tmp_dir: Option<String>,

    /// Log file. Default to /dev/null
    /// When speedtesting, the executed program output is redirected to this file.
    #[clap(long, default_value = "/dev/null")]
    log: String,

    /// Upstream path. Will be given to specified program
    #[clap(value_parser)]
    upstream: String,

    /// Program to use. It will try to detect by default (here curl will be used default for http(s))
    #[clap(long, value_enum)]
    program: Option<Program>,

    /// Extra arguments. Will be given to specified program
    #[clap(long, allow_hyphen_values = true, value_parser = parse_extra)]
    extra: Vec<String>,
}

fn parse_extra(extra: &str) -> Result<Vec<String>, String> {
    shlex::split(extra).map_or_else(|| Err("Failed to parse extra arguments".to_string()), Ok)
}

struct Target {
    network: String,
    comment: String,
}

#[inline]
fn get_program_name(program: Program) -> String {
    match program {
        Program::Rsync => "rsync",
        Program::Wget => "wget",
        Program::Curl => "curl",
        Program::Git => "git",
    }
    .to_owned()
}

fn create_tmp_file(tmp_dir: Option<&String>) -> mktemp::Temp {
    tmp_dir
        .map_or_else(mktemp::Temp::new_file, |tmp_dir| {
            mktemp::Temp::new_file_in(tmp_dir)
        })
        .expect("tmp file created failed")
}

fn create_tmp_dir(tmp_dir: Option<&String>) -> mktemp::Temp {
    tmp_dir
        .map_or_else(mktemp::Temp::new_dir, |tmp_dir| {
            mktemp::Temp::new_dir_in(tmp_dir)
        })
        .expect("tmp dir created failed")
}

struct ProgramStatus {
    status: ExitStatus,
    time: Duration,
}

struct ProgramChild {
    child: process::Child,
    program: Program,
}

fn get_config_paths(args: &Args) -> Vec<PathBuf> {
    if let Some(config) = args.config.as_ref() {
        vec![Path::new(config).to_path_buf()]
    } else {
        let mut paths = Vec::new();
        let xdg_dir = BaseDirectories::new();
        if let Some(xdg_config) = xdg_dir.get_config_file("bestbind.conf").as_ref() {
            paths.push(xdg_config.clone());
        }
        if let Some(home) = dirs::home_dir() {
            let home_path = home.join(".bestbind.conf");
            paths.push(home_path);
        }
        paths.push(Path::new("/etc/bestbind.conf").to_path_buf());

        paths
    }
}

fn get_profile(args: &Args, config: &str) -> Result<Profile> {
    let profiles: HashMap<String, Profile> = toml::from_str(config)?;
    match profiles.get(&args.profile) {
        Some(profile) => {
            if profile.format == Format::Docker && profile.image.is_empty() {
                return Err(anyhow::anyhow!(
                    "Docker format requires 'image' field in profile"
                ));
            }
            Ok(profile.clone())
        }
        None => Err(anyhow::anyhow!(
            "Profile '{}' not found in config file",
            args.profile
        )),
    }
}

fn main() {
    let args = Args::parse();
    let config_paths = get_config_paths(&args);
    let log = File::create(&args.log).expect("Cannot open log file");
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(SIGINT, Arc::clone(&term)).expect("Register SIGINT handler failed");
    signal_hook::flag::register(SIGTERM, Arc::clone(&term))
        .expect("Register SIGTERM handler failed");

    let mut config_file = None;
    let mut error_msgs = Vec::new();
    for config in config_paths {
        match File::open(&config) {
            Ok(file) => {
                config_file = Some(file);
                break;
            }
            Err(e) => {
                error_msgs.push(format!("Tried: {}, got error: {e}", config.display()));
            }
        }
    }
    let Some(mut config_file) = config_file else {
        panic!("Cannot open config file. {}", error_msgs.join("\n"));
    };
    let mut full_config: String = String::new();
    config_file
        .read_to_string(&mut full_config)
        .expect("Cannot read config file");
    let profile =
        get_profile(&args, &full_config).expect("Cannot parse config file or profile not found");

    let program = if let Some(program) = args.program {
        program
    } else {
        // We need to detect by upstream

        // Though I don't think anyone will use ALL UPPERCASE here...
        let upstream = args.upstream.to_lowercase();
        if upstream.starts_with("rsync://") || upstream.contains("::") {
            Program::Rsync
        } else if upstream.starts_with("http://") || upstream.starts_with("https://") {
            if upstream.ends_with(".git") {
                Program::Git
            } else {
                Program::Curl
            }
        } else if upstream.starts_with("git://") {
            Program::Git
        } else {
            panic!("Cannot detect upstream program. Please specify with --program.")
        }
    };

    let runner = get_runner(profile.format, &args, profile, program);
    let uses = runner.uses();

    let mut results: Vec<Vec<_>> = Vec::new();
    for pass in 0..args.pass {
        println!("Pass {pass}:");
        let mut results_pass: Vec<_> = Vec::new();
        for target in uses {
            if term.load(Ordering::SeqCst) {
                println!("Terminated by user.");
                // return instead of directly exit() so we can clean up tmp files
                return;
            }
            // create tmp file or directory
            let tmp_file = if program != Program::Git {
                create_tmp_file(args.tmp_dir.as_ref())
            } else {
                create_tmp_dir(args.tmp_dir.as_ref())
            };
            let mut proc = runner.run(&target.network, &tmp_file, &log);
            let prog_status =
                proc.wait_timeout(Duration::from_secs(args.timeout as u64), term.clone());
            let status = prog_status.status;
            let duration = prog_status.time;
            let duration_seconds = duration.as_secs_f64();
            let mut state_str = {
                if duration_seconds > args.timeout as f64 {
                    format!("✅ {} timeout as expected", get_program_name(program))
                } else {
                    match status.code() {
                        Some(code) => match code {
                            0 => "✅ OK".to_owned(),
                            _ => format!(
                                "❌ {} failed with code {}",
                                get_program_name(program),
                                code
                            ),
                        },
                        None => format!("❌ {} killed by signal", get_program_name(program)),
                    }
                }
            };
            if term.load(Ordering::SeqCst) {
                state_str += " (terminated by user)";
            }
            // check file size
            let size = if program == Program::Git {
                tmp_file.metadata().unwrap().len()
            } else {
                fs_extra::dir::get_size(&tmp_file).unwrap()
            };
            let bandwidth = size as f64 / duration_seconds; // Bytes / Seconds
            let bandwidth = bandwidth / 1024_f64; // KB/s
            println!(
                "{} ({}): {} KB/s ({})",
                target.network, target.comment, bandwidth, state_str
            );
            results_pass.push(bandwidth);
        }
        results.push(results_pass);
    }

    let mut calculated_results: Vec<_> = Vec::new();
    for (i, ip) in uses.iter().enumerate() {
        let mut sum = 0_f64;
        let mut vmin = f64::MAX;
        let mut vmax = f64::MIN;
        for pass in &results {
            let bandwidth = pass[i];
            sum += bandwidth;
            vmin = f64::min(vmin, bandwidth);
            vmax = f64::max(vmax, bandwidth);
        }
        let res = if args.pass >= 3 {
            // Remove min and max
            sum -= vmin + vmax;
            sum / (args.pass - 2) as f64
        } else {
            sum / args.pass as f64
        };
        calculated_results.push((ip.network.clone(), ip.comment.clone(), res));
    }

    println!("Final Results (remove min and max if feasible, and take average):");
    calculated_results.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());
    for (ip, comment, res) in calculated_results {
        println!("{ip} ({comment}): {res} KB/s");
    }
}

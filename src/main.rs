use std::{io::Write, error::Error, fmt, collections::LinkedList, thread, time, sync::{Arc, atomic::AtomicBool}};
use clap::Parser;
use byte_unit::Byte;
use log::{debug, error, info, warn};
use sysinfo::{Pid, System, MemoryRefreshKind};
use signal_hook::{consts::SIGTERM, iterator::Signals};
use libsystemd::daemon;

#[derive(Debug)]
struct ProcessReaperError {
    text: String
}

impl ProcessReaperError {
    fn new(text: &str) -> ProcessReaperError {
        ProcessReaperError{text: text.to_string()}
    }
}

impl Error for ProcessReaperError {}

impl fmt::Display for ProcessReaperError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Error: {}", self.text)
    }
}

#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Args {
    /// Name of process(es) to monitor
    #[clap(short, long)]
    process_name: String,

    /// Memory limit; if exceeded, offending process will be killed
    #[clap(short, long)]
    memory_limit: String,

    /// Use syslog
    #[clap(long, action)]
    syslog: bool,

    /// Notify systemd for watchdog compatibility
    #[clap(long, action)]
    systemd_notify: bool
}

fn main() -> Result<(), Box<dyn Error>> {
    let args = Args::parse();
    init_logging(args.syslog);

    info!("Initializing");
    let (should_run, mut sys, mem_limit) = initialize(&args.memory_limit)
        .expect("Failed to initialize");

    info!("Entering monitoring loop; target process: {}", args.process_name);
    if args.systemd_notify {
        debug!("Notifying systemd that daemon is ready");
        let _ = daemon::notify(
            false,
            &[
                daemon::NotifyState::Ready,
                daemon::NotifyState::Status(format!("Monitoring {} (limit {})", args.process_name, args.memory_limit).into()),
            ],
        );
    }

    let mut loop_number = 0_u64;
    while should_run.load(std::sync::atomic::Ordering::Relaxed) {
        debug!("Starting loop #{}", loop_number);
        loop_number += 1;

        let mut termed_pids: LinkedList<Pid> = LinkedList::new();
        sys.refresh_all();
        let target_processes = sys.processes_by_exact_name(&args.process_name);
        for process in target_processes {
            let mem_usage = process.memory();
            let pid = process.pid();
            let mem_usage_str = Byte::from_u64(mem_usage)
                .get_appropriate_unit(byte_unit::UnitType::Binary)
                .to_string();

            if mem_usage >= mem_limit {
                warn!("{} ({}) memory usage of {} greater than threshold of {}; terminating", 
                    args.process_name, pid, mem_usage_str, args.memory_limit);
                let sig_sent = 
                    process.kill_with(sysinfo::Signal::Term)
                        .expect("sysinfo::Signal::Term signal doesn't exist on this system");

                if !sig_sent {
                    error!("Failed to send sysinfo::Signal::Term to {} ({})", args.process_name, pid);
                }

                termed_pids.push_back(pid);
            }
            else {
                debug!("{} ({}) using {} of memory", args.process_name, pid, mem_usage_str);
            }
        }

        let sleep_dur = time::Duration::from_secs(2);
        debug!("Sleeping for {} seconds", sleep_dur.as_secs_f32());
        thread::sleep(sleep_dur);

        sys.refresh_all();
        for pid in termed_pids {
            if let Some(process) = sys.process(pid) {
                warn!("Terminated process, {} ({}), still alive; killing", args.process_name, pid);
                let sig_sent = 
                    process.kill_with(sysinfo::Signal::Kill)
                        .expect("sysinfo::Signal::Kill signal doesn't exist on this system");

                if !sig_sent {
                    error!("Failed to send sysinfo::Signal::Kill to {} ({})", args.process_name, pid);
                }
            }
            else {
                debug!("Could not find {} ({}) again; assuming successful termination", args.process_name, pid)
            }
        }

        if args.systemd_notify {
            debug!("Petting systemd watchdog");
            daemon::notify(false, &[daemon::NotifyState::Watchdog])
                .expect("Failed to pet systemd watchdog");
        }
    }

    Ok(())
}

fn init_logging(use_syslog: bool) {
    let mut log_builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"));

    if use_syslog {
        log_builder.format(|buffer, record| {
            writeln!(buffer, "<{}>{}", record.level() as u8 + 2 , record.args())
        });
    }

    log_builder.init();
}

fn initialize(memory_limit: &str) -> Result<(Arc<AtomicBool>, System, u64), Box<dyn Error>> {
    let should_run = Arc::new(AtomicBool::new(true));
    let should_run_arc_clone = Arc::clone(&should_run);

    let mut signals = Signals::new([SIGTERM])?;
    thread::spawn(move || {
        let should_run = should_run_arc_clone;
        for sig in signals.forever() {
            warn!("Received signal {:?}", sig);
            should_run.store(false, std::sync::atomic::Ordering::Relaxed);
        }
    });

    let mut sys = System::new();
    
    sys.refresh_memory_specifics(MemoryRefreshKind::new().with_ram());
    let mem_limit = str_to_bytes_of_memory(memory_limit, &sys)?;

    Ok((should_run, sys, mem_limit))
}

fn str_to_bytes_of_memory(mem_str: &str, sys: &System) -> Result<u64, Box<dyn Error>> {
    if mem_str.contains('%') {
        let total_mem = sys.total_memory();
        let mem_ratio = mem_str.replace('%', "");
        let mem_ratio = mem_ratio.parse::<f32>()? / 100.0_f32;
        if mem_ratio >= 1.0 {
            return Err(Box::new(ProcessReaperError::new("Memory percentage >= 100%")))
        }
        let mem_bytes = ((total_mem as f32) * mem_ratio).round() as u64;
        Ok(mem_bytes)
    }
    else {
        let mem_bytes = Byte::parse_str(mem_str, true)?.as_u64();
        Ok(mem_bytes)
    }
}

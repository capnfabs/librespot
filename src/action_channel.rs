use std::{io::BufRead, thread::JoinHandle, sync::{atomic::Ordering, Arc}, os::unix::fs::OpenOptionsExt};

use nix::libc;
use tokio::sync::mpsc::{self, UnboundedReceiver};
use core::sync::atomic::AtomicBool;


#[derive(Debug)]
pub enum Command {
    Unknown,
    Prev,
    Next,
    PlayPause,
    VolUp,
    VolDown,
    Load {spotify_id: String},
}

pub struct ActionChannelTask {
    handle: JoinHandle<()>,
    shutdown_started: Arc<AtomicBool>,
}

fn parse_line(line: String) -> Command {
    match line.as_str() {
        "PlayPause" => Command::PlayPause,
        "Next" => Command::Next,
        "Prev" => Command::Prev,
        "VolUp"=> Command::VolUp,
        "VolDown"=> Command::VolDown,
        load_line if line.starts_with("load:") => Command::Load { spotify_id: load_line.trim_start_matches("load:").to_string() },
        _ => Command::Unknown,
    }
}

fn pipe_path() -> String {
    let uid = unsafe { nix::libc::geteuid() };
    format!("/run/user/{uid}/librespot-commands.pipe")
}

/*
Ok, let's talk about this obscene construction.
There's (I think): something wrong in the implementation of tokio::fs::File.open such that
it ends up being blocking. When you use the current_thread loop thingy, it means that that
blocking thread ends up staying open forever.
Blocking while files open isn't usually much of a problem because it doesn't take ages?
The exception is named pipes though, which stay block on open indefinitely until a
reader / writer joins.

So this mad construction is: we just ignore tokio for the "action channel", and when we
shut down, we still need to ensure that we unblock the reader. So, we flip a bool flag,
then open the file a few times in nonblocking mode and shut them again immediately
to unblock the rest of the loop and shut everything down. Gross.
*/
impl ActionChannelTask {
    pub fn shutdown(self) {
        self.shutdown_started.store(true, Ordering::SeqCst);
        let path = pipe_path();
        for _i in 0..3 {
            // ignore errors lol
            let _ = std::fs::File::options().truncate(true).write(true).custom_flags(libc::O_NONBLOCK).open(&path);
        }
        self.handle.join().unwrap();
    }

    pub fn new() -> (ActionChannelTask, UnboundedReceiver<Command>) {
        let (sender, receiver) = mpsc::unbounded_channel();

        let shutdown_started = Arc::new(AtomicBool::new(false));
        let shutdown_started_2 = shutdown_started.clone();

        let handle = std::thread::spawn(move || {
            let path = pipe_path();
            match nix::unistd::mkfifo(&path[..], nix::sys::stat::Mode::S_IRWXU) {
                Err(e) if e == nix::errno::Errno::EEXIST => Ok(()),
                r => r
            }.unwrap();
            while !shutdown_started.load(Ordering::SeqCst) {
                let file: std::fs::File = std::fs::File::open(&path).unwrap();
                let reader = std::io::BufReader::new(file);
                let lines = reader.lines();
                for line in lines {
                    sender.send(parse_line(line.unwrap())).unwrap();
                }
            }
            println!("Exited command loop");
        });
        let task = ActionChannelTask{
            handle,
            shutdown_started: shutdown_started_2,
        };
        (task, receiver)
    }
}


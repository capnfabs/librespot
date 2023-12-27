use std::io;

use tokio::{sync::{mpsc::{self, UnboundedReceiver, UnboundedSender}, watch}, net::{TcpListener, TcpStream, ToSocketAddrs}, select, io::{BufReader, AsyncBufReadExt}};


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

async fn process_socket(stream: TcpStream, sender: UnboundedSender<Command>) -> io::Result<()> {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    while let Some(line) = lines.next_line().await? {
        sender.send(parse_line(line)).unwrap();
    }
    Ok(())
}

pub struct ActionChannelTask<A>
where A: ToSocketAddrs {
    addr: A,
    sender: UnboundedSender<Command>,
    shutdown_started_tx: watch::Sender<bool>,
    shutdown_started_rx: watch::Receiver<bool>,
}

impl<A> ActionChannelTask<A>
where A: ToSocketAddrs {

    pub fn shutdown(self) {
        self.shutdown_started_tx.send(true).unwrap();
    }

    pub async fn listen(&self) -> io::Result<()> {
        if *self.shutdown_started_rx.borrow() {
            return Ok(());
        }
        let listener = TcpListener::bind(&self.addr).await?;

        let mut shutdown_started = self.shutdown_started_rx.clone();

        loop {
            select! {
                res = listener.accept() => {
                    let (socket, _addr) = res.unwrap();
                    let sender = self.sender.clone();
                    tokio::spawn(async move {
                        process_socket(socket, sender).await.unwrap();
                    });
                },
                _ = shutdown_started.changed() => {
                    break;
                },
            }
        }
        Ok(())
    }

    pub fn new(addr: A) -> (ActionChannelTask<A>, UnboundedReceiver<Command>) {
        let (sender, receiver) = mpsc::unbounded_channel();
        let (shutdown_started_tx, shutdown_started_rx) = watch::channel(false);

        let task = ActionChannelTask{
            addr,
            sender,
            shutdown_started_tx,
            shutdown_started_rx,
        };
        (task, receiver)
    }
}


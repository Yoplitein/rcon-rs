#![allow(unused, non_snake_case)]

use std::{borrow::Borrow, io::Write, time::Duration};

use clap::Parser;
use anyhow::{anyhow, Result as AResult};
use tokio::{net::UdpSocket, time::timeout};

#[derive(Debug, Parser)]
struct Args {
    #[arg(short = 'H', long, default_value = "127.0.0.1")]
    host: String,
    
    #[arg(short = 'P', long, default_value = "27015")]
    port: u16,
    
    #[arg(short = 'p', long)]
    password: String,
    
    #[arg(short, long, default_value = "source")]
    mode: Mode,
    
    commands: Vec<String>,
}

#[derive(Clone, Copy, Debug, clap::ValueEnum)]
enum Mode {
    Goldsrc,
    #[value(alias = "minecraft")]
    Source,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> AResult<()> {
    let args = Args::parse();
    dbg!(&args);
    
    match args.mode {
        Mode::Goldsrc => {
            let sock = UdpSocket::bind("0.0.0.0:0").await?;
            sock.connect((args.host, args.port)).await?;
            let rcon = GoldsrcRcon::new(args.password, sock);
            
            for command in args.commands {
                let resp = rcon.send_command(&command).await?;
                eprintln!("{resp}");
            }
        },
        Mode::Source => {
            todo!()
        },
    }
    Ok(())
}

struct GoldsrcRcon {
    password: String,
    socket: UdpSocket,
}

impl GoldsrcRcon {
    pub fn new(password: String, socket: UdpSocket) -> Self {
        Self {
            password,
            socket,
        }
    }
    
    async fn send_raw(&self, bytes: &[u8]) -> AResult<String> {
        self.socket.send(bytes).await?;
        
        let mut buf = [0u8; 8192];
        let mut result = String::new();
        let mut haveFirstChunk = false;
        loop {
            let len = match timeout(Duration::from_secs(1), self.socket.recv(&mut buf)).await {
                Ok(res) => res?,
                Err(err) => if haveFirstChunk {
                    break;
                } else {
                    return Err(err)?;
                },
            };
            
            // protocol adds miscellaneous padding bytes for inscrutable reasons
            let mut trimBuf = buf[.. len].to_owned();
            let mut start = 'start: {
                for (i, v) in trimBuf.iter().copied().enumerate() {
                    if !matches!(v, 0xFF | 0xFE | 0x1D | 0x1C | 0x00) {
                        break 'start i;
                    }
                }
                trimBuf.len()
            };
            if haveFirstChunk {
                start += 1;
            }
            let end = 'end: {
                for (i, v) in trimBuf.iter().copied().enumerate().rev() {
                    if v != 0x00 {
                        break 'end i;
                    }
                }
                trimBuf.len()
            };
            trimBuf.drain(end ..);
            trimBuf.drain(.. start.min(trimBuf.len()));
            
            let mut str = String::from_utf8_lossy(trimBuf.as_slice());
            result.push_str(&str);
            haveFirstChunk = true;
        }
        Ok(result)
    }
    
    async fn get_challenge(&self) -> AResult<String> {
        let challengeBytes = b"\xff\xff\xff\xffchallenge rcon";
        let challenge = self.send_raw(challengeBytes).await?;
        let challenge = challenge.split(" ").last().ok_or_else(|| anyhow!("got empty challenge"))?.trim();
        Ok(challenge.into())
    }
    
    pub async fn send_command(&self, command: &str) -> AResult<String> {
        let challenge = self.get_challenge().await?;
        let mut buf = Vec::with_capacity(1024);
        buf.extend([0xff; 4]);
        write!(&mut buf, "rcon {challenge} \"{}\" {command}\x00", self.password)?;
        let mut resp = self.send_raw(&buf).await?;
        resp.remove(0); // command responses are prefixed with an `l`
        Ok(resp)
    }
}

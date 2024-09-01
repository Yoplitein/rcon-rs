#![allow(unused, non_snake_case)]

use std::io::{self, stdin, BufRead, Write};
use std::time::Duration;

use anyhow::{anyhow, Context, Result as AResult};
use clap::Parser;
use tokio::io::{AsyncReadExt, AsyncWriteExt, BufStream};
use tokio::net::{TcpStream, UdpSocket};
use tokio::time::timeout;

#[derive(Debug, Parser)]
struct Args {
	#[arg(short = 'H', long, default_value = "127.0.0.1")]
	host: String,

	#[arg(short = 'P', long)]
	port: Option<u16>,

	#[arg(short = 'p', long)]
	password: String,

	#[arg(short, long, default_value = "source")]
	game: Game,

	commands: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
enum Game {
	Goldsrc,
	Source,
	Minecraft,
	Factorio,
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> AResult<()> {
	let mut args = Args::parse();

	args.port.get_or_insert(match args.game {
		Game::Goldsrc | Game::Source => 27015,
		Game::Minecraft => 25575,
		Game::Factorio => 0, // FIXME
	});

	#[cfg(debug_assertions)]
	dbg!(&args);

	macro_rules! inner_loop {
		($cmd:ident, $impl:tt) => {
			if args.commands.is_empty() {
				let (mut sender, mut receiver) = tokio::sync::mpsc::channel(16);
				std::thread::spawn(move || {
					let mut stdin = stdin().lock();
					for line in stdin.lines() {
						sender.blocking_send(line);
					}
				});
				while let Some(Ok($cmd)) = receiver.recv().await {
					$impl
				}
			} else {
				for $cmd in args.commands {
					$impl
				}
			}
		};
	}

	match args.game {
		Game::Goldsrc => {
			let sock = UdpSocket::bind("0.0.0.0:0").await?;
			sock.connect((args.host, args.port.unwrap())).await?;
			let rcon = GoldsrcRcon::new(args.password, sock);
			inner_loop!(command, {
				let resp = rcon.send_command(&command).await?;
				if !resp.is_empty() {
					println!("{resp}");
				}
			});
		},
		Game::Source | Game::Minecraft | Game::Factorio => {
			let sock = TcpStream::connect((args.host, args.port.unwrap())).await?;
			let mut rcon = SourceRcon::new(sock, args.game);
			rcon.login(&args.password).await?;
			inner_loop!(command, {
				let resp = rcon.send_command(&command).await?;
				if !resp.is_empty() {
					println!("{resp}");
				}
			});
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
		Self { password, socket }
	}

	async fn send_raw(&self, bytes: &[u8]) -> AResult<String> {
		self.socket.send(bytes).await?;

		let mut buf = [0u8; 8192];
		let mut result = String::new();
		let mut haveFirstChunk = false;
		loop {
			let len = match timeout(Duration::from_secs(1), self.socket.recv(&mut buf)).await {
				Ok(res) => res?,
				Err(err) => {
					if haveFirstChunk {
						break;
					} else {
						return Err(err)?;
					}
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
			start += 1; // responses seem to always be prefixed with 'l'
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
		let challenge = challenge
			.split(" ")
			.last()
			.ok_or_else(|| anyhow!("got empty challenge"))?
			.trim();
		Ok(challenge.into())
	}

	pub async fn send_command(&self, command: &str) -> AResult<String> {
		let challenge = self.get_challenge().await?;
		let mut buf = Vec::with_capacity(1024);
		buf.extend([0xFF; 4]);
		write!(
			&mut buf,
			"rcon {challenge} \"{}\" {command}\x00",
			self.password
		)?;
		let mut resp = self.send_raw(&buf).await?;
		Ok(resp)
	}
}

struct SourceRcon {
	socket: BufStream<TcpStream>,
	id: i32,
	game: Game,
}

impl SourceRcon {
	pub fn new(socket: TcpStream, game: Game) -> Self {
		// this is required as Minecraft ignores commands if the entire packet isn't
		// written at once >:| but it's also a minor perfomance win so *shrug*
		let socket = BufStream::with_capacity(8192, 8192, socket);

		let mut this = Self {
			socket,
			id: 0,
			game,
		};
		this
	}

	async fn send_packet(&mut self, ty: i32, body: &[u8]) -> AResult<i32> {
		let id = self.id;
		self.id += 1;

		let len = 10 + body.len();
		self.socket.write_i32_le(len as i32).await?;
		self.socket.write_i32_le(id).await?;
		self.socket.write_i32_le(ty).await?;
		self.socket.write_all(body).await?;
		self.socket.write_all(&[0, 0]).await?;
		self.socket.flush().await?;

		Ok(id)
	}

	async fn recv_packet(&mut self) -> AResult<(i32, i32, Vec<u8>)> {
		let len = self.socket.read_i32_le().await?;
		let id = self.socket.read_i32_le().await?;
		let ty = self.socket.read_i32_le().await?;

		let mut body = vec![0; (len - 10) as usize];
		self.socket.read_exact(&mut body).await?;

		let mut trailing = [0; 2];
		self.socket.read_exact(&mut trailing).await?;

		Ok((id, ty, body))
	}

	pub async fn login(&mut self, password: &str) -> AResult<()> {
		self.send_packet(3, password.as_bytes()).await?;

		if matches!(self.game, Game::Source) {
			// Source first sends an empty response, then authentication response packet
			// Minecraft elides this
			let (_, ty, _) = timeout(Duration::from_secs(1), self.recv_packet())
				.await
				.context("timed out waiting for response from server")??;
			if ty != 0 {
				return Err(anyhow!(
					"server sent unexpected packet during authentication"
				));
			}
		}

		let (id, ty, _) = timeout(Duration::from_secs(1), self.recv_packet())
			.await
			.context("timed out waiting for response from server")??;
		if ty != 2 {
			return Err(anyhow!(
				"server sent unexpected packet during authentication"
			));
		}
		if id == -1 {
			return Err(anyhow!("authentication failed, bad password?"));
		}

		Ok(())
	}

	pub async fn send_command(&mut self, command: &str) -> AResult<String> {
		let id = self.send_packet(2, command.as_bytes()).await?;

		// output may be split between several response packets, so we send a bogus
		// packet that generates a reply which arrives only after the final split packet
		// has been received
		let finishedId = if matches!(self.game, Game::Factorio) {
			self.send_packet(2, b"/bogus")
		} else {
			self.send_packet(0, b"")
		}
		.await?;

		let mut response = String::new();
		loop {
			let resp = timeout(Duration::from_secs(1), self.recv_packet())
				.await
				.context("timed out waiting for response from server")??;
			if resp.0 != id || resp.1 != 0 {
				if resp.0 == finishedId {
					break;
				} else {
					return Err(anyhow!("server sent unexpected response packet"));
				}
			}
			response.push_str(&String::from_utf8_lossy(&resp.2));
		}

		let trimmed = response.trim_end();
		response.drain(trimmed.len() ..);

		Ok(response)
	}
}

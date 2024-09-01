# rcon-rs
A remote console (RCON) client supporting GoldSrc, Source, Minecraft, and Factorio.

```
Usage: rcon-rs [OPTIONS] --password <PASSWORD> [COMMANDS]...

Arguments:
  [COMMANDS]...
          List of commands to execute.

          Mind your shell's argument splitting!

Options:
  -H, --host <HOST>
          Hostname or address to connect to.

          Note that you should avoid using RCON over the internet as there is no encryption.

          [default: 127.0.0.1]

  -P, --port <PORT>
          Port to connect on.

          If unspecified will use the default for GoldSrc, Source, and Minecraft.
          Factorio has no default and therefore must always be specified.

  -p, --password <PASSWORD>
          RCON password

  -g, --game <GAME>
          Game being connected to

          [default: source]
          [possible values: goldsrc, source, minecraft, factorio]

  -t, --timeout <TIMEOUT>
          Time (in seconds) to wait for a response from the server.

          For GoldSrc, this also controls how long to wait for the entirety of a split response to
          arrive, i.e. each command will take at least this much time to execute.

          [default: 1]

  -h, --help
          Print help (see a summary with '-h')
```
